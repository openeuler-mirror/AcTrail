"""Daemon shutdown with a real pending MCP call and failing trace-close persistence."""

import argparse
import json
import os
import secrets
import shutil
import signal
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

from scripts.bench.payload.action_state.workload import McpAgentWorkload
from scripts.bench.payload.measurement import DaemonCpu
from scripts.bench.payload.runtime import CollectionRuntime
from tests.v2.common.mcp_test_support.probe import McpProbeWorkspace
from tests.v2.common.test_suite_tools.mcp.mcp_probe_server import (
    EventRecorder, McpApplication, StdioMcpServer,
)

from .environment import DeliveryEnvironment


ROOT = Path(__file__).resolve().parents[4]
FAULT = "acceptance:trace_closed_storage_failure"


class PendingToolApplication(McpApplication):
    def handle(self, message):
        response = super().handle(message)
        if message.get("method") == "tools/call" and response and "result" in response:
            # The real tool executes and records its marker, but never responds.
            # A deadline bounds orphan lifetime if fixture cleanup itself fails.
            time.sleep(60)
            raise TimeoutError("pending tool was not terminated by its owning fixture")
        return response


class PendingToolWorkspace(McpProbeWorkspace):
    def stdio_command(self, spec):
        return str(Path(sys.executable).resolve()), [
            "-m", "scripts.bench.payload.storage_delivery.finalization", "--server",
            "--server-name", spec.server_name, "--tool-name", spec.tool_name,
            "--marker", spec.marker, "--event-log", str(spec.event_log),
        ]


class FinalizationAcceptance:
    def __init__(self, args):
        self.output = args.output_dir.resolve()
        self.bins = args.bin_dir.resolve()
        self.binary = args.agent_bin.resolve()

    def run(self):
        self.output.mkdir(parents=True, exist_ok=False)
        environment = DeliveryEnvironment(ROOT, self.bins, self.output / "runtime", "finalization")
        probe = PendingToolWorkspace(ROOT, self.output, "mcp")
        spec = probe.spec(server_name="close_probe", tool_name="emit_marker",
                          marker="CLOSE_" + secrets.token_hex(8))
        agent = McpAgentWorkload(ROOT, self.output / "maas", self.binary,
            probe=probe, spec=spec, model_tool_name=spec.tool_id, tpot_ms=3, timeout_seconds=60)
        result = {"status": "running", "scenario": "shutdown-trace-close-update"}
        process = None
        collection = None
        try:
            environment.prepare()
            config = environment.current_config()
            config["action_kinds"]["mcp.tool_call"] = True
            environment.update_config(config)
            pid = int((environment.config.work_dir / "run/actraild.pid").read_text())
            if Path(f"/proc/{pid}/exe").resolve() != self.bins / "actraild":
                raise RuntimeError("owned daemon does not match requested binaries")
            collection = CollectionRuntime(environment.config.work_dir, self.bins, environment.patch,
                {"drain_timeout_seconds": 30, "poll_seconds": 0.02})
            collection.cpu = DaemonCpu(pid)
            collection.log = (environment.config.work_dir / "log/actraild.log").open(errors="replace")
            previous = collection.mark()
            sql = (
                "CREATE TRIGGER acceptance_trace_closed_failure BEFORE UPDATE ON semantic_action_state "
                "WHEN NEW.finalization_reason=1 AND NEW.action_key IN "
                f"(SELECT action_key FROM semantic_actions WHERE trace_id>{previous} AND kind_code=121) "
                f"BEGIN SELECT RAISE(ABORT, '{FAULT}'); END;"
            )
            (self.output / "fault.sql").write_text(sql + "\n")
            with sqlite3.connect(environment.database, timeout=5) as database:
                database.execute(sql)
            agent.start()
            task = self.output / "agent"
            agent.prepare(task)
            agent.reset()
            command = collection.launch(agent.command(task))
            (self.output / "command.json").write_text(json.dumps(command, indent=2) + "\n")
            env = dict(os.environ, **agent.env)
            env["PYTHONPATH"] = os.pathsep.join(filter(None, [str(ROOT), env.get("PYTHONPATH")]))
            with (task / "stdout.log").open("wb") as stdout, (task / "stderr.log").open("wb") as stderr:
                process = subprocess.Popen(command, cwd=task, env=env,
                    stdout=stdout, stderr=stderr, start_new_session=True)
                result["pending"] = self.wait_pending(process, collection, probe, spec, previous)
                # Shutdown finalizes pending semantics while the MCP process is alive.
                # Process-exit finalization is a separate scenario.
                stopped = environment.runtime.stop()
                if stopped is None or stopped.returncode:
                    raise RuntimeError("owned daemon did not shut down successfully")
                environment._plugin_loaded = False
                result["shutdown"] = {"exit_code": stopped.returncode,
                                      "agent_alive_after_shutdown": process.poll() is None}
                result["online"] = self.wait_terminal(environment)
                self.stop_owned_group(process)
            result["agent_exit_code"] = process.returncode
            log = (environment.config.work_dir / "log/actraild.log").read_text(errors="replace")
            if FAULT not in log:
                raise RuntimeError("TraceClosed state UPDATE failure was not observed")
            states = collection.query(
                "SELECT a.action_key,s.end_time,s.finalization_reason FROM semantic_actions a "
                "JOIN semantic_action_state s ON s.action_key=a.action_key "
                "WHERE a.trace_id>? AND a.kind_code=121", (previous,))
            if len(states) != 1 or states[0][1] is not None or states[0][2] is not None:
                raise RuntimeError(f"failed finalization update unexpectedly persisted: {states}")
            events = [json.loads(line) for line in spec.event_log.read_text().splitlines()]
            request_id = result["pending"]["request_id"]
            if any(row.get("direction") == "server_to_client"
                   and row.get("message", {}).get("id") == request_id for row in events):
                raise RuntimeError("pending MCP tool unexpectedly returned a response")
            result.update(status="passed", fault_log_verified=True, persisted_states=states,
                          mcp_response_absent=True, probe_events=str(spec.event_log))
        except BaseException as error:
            result.update(status="failed", error=f"{type(error).__name__}: {error}")
            raise
        finally:
            try:
                if process and process.poll() is None:
                    self.stop_owned_group(process)
                agent.stop()
                if collection and collection.log:
                    collection.log.close()
                environment.close()
            except BaseException as error:
                result.update(status="failed", cleanup_error=f"{type(error).__name__}: {error}")
                raise
            finally:
                (self.output / "acceptance.json").write_text(json.dumps(result, indent=2) + "\n")

    @staticmethod
    def wait_pending(process, collection, probe, spec, previous):
        deadline = time.monotonic() + 40
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise RuntimeError(f"agent exited before pending MCP call: {process.returncode}")
            collection.cpu.read_ms()
            if probe.execution_count(spec) == 1:
                events = [json.loads(line) for line in spec.event_log.read_text().splitlines()]
                execution = next(row for row in events if row.get("event") == "tool_execution")
                if execution["arguments"] != {"marker": spec.marker}:
                    raise RuntimeError("real MCP tool executed unexpected arguments")
                if os.getpgid(execution["pid"]) != process.pid:
                    raise RuntimeError("MCP child is outside fixture-owned process group")
                states = collection.query(
                    "SELECT s.end_time FROM semantic_actions a JOIN semantic_action_state s "
                    "ON s.action_key=a.action_key WHERE a.trace_id>? AND a.kind_code=121", (previous,))
                if states == [(None,)]:
                    return {"mcp_executions": 1, "request_id": execution["request_id"],
                            "server_pid": execution["pid"], "owned_process_group": process.pid}
            time.sleep(0.05)
        raise TimeoutError("real agent did not create a pending MCP tool call")

    @staticmethod
    def stop_owned_group(process):
        if process.poll() is not None:
            return
        if os.getpgid(process.pid) != process.pid:
            raise RuntimeError("fixture process no longer owns its process group")
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)

    @staticmethod
    def wait_terminal(environment):
        deadline = time.monotonic() + 10
        observed = []
        while time.monotonic() < deadline:
            observed = [environment.attributes(span) for span in environment.spans()
                        if environment.attributes(span).get("actrail.action.kind") == "mcp.tool_call"]
            terminal = [attrs for attrs in observed
                        if attrs.get("actrail.action.status") == "error"
                        and attrs.get("actrail.action.completeness") == "partial"]
            if terminal:
                ids = {attrs["actrail.action.id"] for attrs in terminal}
                if len(ids) != 1:
                    raise RuntimeError(f"multiple finalized MCP tool identities: {ids}")
                return {"transport": "live OTLP/HTTP JSON", "terminal_actions": terminal}
            time.sleep(0.05)
        raise RuntimeError(f"missing live TraceClosed MCP finalization: {observed}")

    def verify_saved(self):
        """Check durable artifacts from an already completed real workload."""
        path = self.output / "acceptance.json"
        result = json.loads(path.read_text())
        if result.get("shutdown", {}).get("exit_code") != 0:
            raise RuntimeError("recorded daemon shutdown did not succeed")
        documents = json.loads((self.output / "runtime/online-otlp.json").read_text())
        spans = [span for document in documents for resource in document.get("resourceSpans", [])
                 for scope in resource.get("scopeSpans", []) for span in scope.get("spans", [])]
        terminal = [DeliveryEnvironment.attributes(span) for span in spans]
        terminal = [attrs for attrs in terminal if attrs.get("actrail.action.kind") == "mcp.tool_call"
                    and attrs.get("actrail.action.status") == "error"
                    and attrs.get("actrail.action.completeness") == "partial"]
        if len(terminal) != 1:
            raise RuntimeError(f"expected one recorded online terminal MCP action: {terminal}")
        log = (self.output / "runtime/log/actraild.log").read_text(errors="replace")
        if FAULT not in log:
            raise RuntimeError("recorded TraceClosed trigger did not fire")
        database_path = self.output / "runtime/data/actrail.sqlite"
        with sqlite3.connect(f"file:{database_path}?mode=ro", uri=True) as database:
            trigger = database.execute(
                "SELECT sql FROM sqlite_master WHERE type='trigger' "
                "AND name='acceptance_trace_closed_failure'"
            ).fetchone()
            states = database.execute(
                "SELECT a.action_key,s.end_time,s.finalization_reason FROM semantic_actions a "
                "JOIN semantic_action_state s ON s.action_key=a.action_key WHERE a.kind_code=121"
            ).fetchall()
        if not trigger or "NEW.finalization_reason=1" not in trigger[0]:
            raise RuntimeError("recorded trigger does not select actual TraceClosed updates")
        if len(states) != 1 or states[0][1:] != (None, None):
            raise RuntimeError(f"failed finalization unexpectedly persisted: {states}")
        logs = list(self.output.glob("mcp-*/*.events.jsonl"))
        if len(logs) != 1:
            raise RuntimeError(f"expected one MCP execution log: {logs}")
        events = [json.loads(line) for line in logs[0].read_text().splitlines()]
        executed = [row for row in events if row.get("event") == "tool_execution"]
        if len(executed) != 1 or executed[0]["request_id"] != result["pending"]["request_id"]:
            raise RuntimeError("missing matching real MCP execution evidence")
        if any(row.get("direction") == "server_to_client"
               and row.get("message", {}).get("id") == executed[0]["request_id"] for row in events):
            raise RuntimeError("pending MCP request received a response")
        result.pop("error", None)
        result.update(status="passed", fault_log_verified=True, persisted_states=states,
                      mcp_response_absent=True, probe_events=str(logs[0]),
                      fault={"trigger_sql": trigger[0], "observed_log_marker": FAULT},
                      online={"transport": "live OTLP/HTTP JSON", "terminal_actions": terminal})
        (self.output / "verification.json").write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", action="store_true")
    parser.add_argument("--server-name")
    parser.add_argument("--tool-name")
    parser.add_argument("--marker")
    parser.add_argument("--event-log", type=Path)
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/release")
    parser.add_argument("--output-dir", type=Path)
    parser.add_argument("--verify-existing", action="store_true")
    parser.add_argument("--agent-bin", type=Path, default=Path(shutil.which("xiaoo") or "xiaoo"))
    args = parser.parse_args()
    if args.server:
        recorder = EventRecorder(args.event_log)
        try:
            app = PendingToolApplication(args.server_name, args.tool_name, args.marker, 0, recorder)
            raise SystemExit(StdioMcpServer(app, recorder).run())
        finally:
            recorder.close()
    else:
        if args.output_dir is None:
            parser.error("--output-dir is required")
        acceptance = FinalizationAcceptance(args)
        if args.verify_existing:
            acceptance.verify_saved()
        else:
            acceptance.run()
