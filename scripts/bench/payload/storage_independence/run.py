"""Real xiaoo or OpenCode sessions with online OTLP and trace completion evidence."""

import argparse
import json
import os
import re
import secrets
import shutil
import signal
import subprocess
import time
import tomllib
from pathlib import Path

from scripts.bench.payload.action_state.workload import McpAgentWorkload
from scripts.bench.payload.agent import AgentWorkload
from scripts.bench.overall.runtime.config_patch import ConfigPatch
from scripts.bench.payload.measurement import DaemonCpu
from scripts.bench.payload.storage_delivery.environment import DeliveryEnvironment
from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core import TestOutput
from tests.v2.common.mcp_test_support.probe import McpProbeWorkspace


ROOT = Path(__file__).resolve().parents[4]
MCP_KINDS = {"mcp.tool_call", "mcp.request", "mcp.response", "mcp.stdin", "mcp.stdout"}


class AcceptanceRuntime(ActrailRuntime):
    def __init__(self, bins, config, patch, backend, config_patch):
        super().__init__(ROOT, bins, 60, TestOutput(), config, patch,
                         clean_control_state=False)
        self.patch = patch
        self.backend = backend
        self.config_patch = config_patch

    def prepare(self):
        with self.patch.open("a") as patch:
            patch.write(f'\n[storage]\nbackend = "{self.backend}"\n')
        if self.config_patch:
            ConfigPatch(self.config_patch).apply_isolation(self.patch)
        return super().prepare()


class AcceptanceEnvironment(DeliveryEnvironment):
    def __init__(self, bins, work, backend, config_patch):
        super().__init__(ROOT, bins, work, "normal")
        self.backend = backend
        self.runtime = AcceptanceRuntime(bins, self.operator_config, self.patch,
                                         backend, config_patch)

    def prepare(self):
        super().prepare()
        candidate = self.current_config()
        for kind in MCP_KINDS:
            if kind not in candidate["action_kinds"]:
                raise RuntimeError(f"online exporter does not expose {kind}")
            candidate["action_kinds"][kind] = True
        self.update_config(candidate)
        effective = tomllib.loads(self.operator_config.read_text())
        if effective["storage"]["backend"] != self.backend:
            raise RuntimeError("effective storage backend differs from selected backend")
        self.require_no_database()

    def require_no_database(self):
        if self.backend != "noop":
            return
        for path in (self.database, Path(str(self.database) + "-wal"),
                     Path(str(self.database) + "-shm")):
            if path.exists():
                raise RuntimeError(f"NoOp created a main observation database: {path}")


class StorageAcceptance:
    def __init__(self, args):
        self.args = args
        self.output = args.output_dir.resolve()
        self.bins = args.bin_dir.resolve()

    def run(self):
        self.output.mkdir(parents=True, exist_ok=False)
        if self.args.config_patch:
            shutil.copyfile(self.args.config_patch, self.output / "selected.patch.toml")
        environment = AcceptanceEnvironment(self.bins, self.output / "runtime",
            self.args.storage_backend, self.args.config_patch)
        binary = self.args.agent_bin or Path(shutil.which(self.args.agent_kind)
                                            or self.args.agent_kind)
        result = {"status": "running", "backend": self.args.storage_backend,
                  "agent_kind": self.args.agent_kind,
                  "configuration": ("fresh defaults + selected patch"
                                    if self.args.config_patch else "fresh defaults"),
                  "config_patch": str(self.args.config_patch.resolve()) if self.args.config_patch else None,
                  "consumer": "online OTLP/HTTP JSON"}
        if self.args.agent_kind == "xiaoo":
            probe = McpProbeWorkspace(ROOT, self.output, "mcp")
            spec = probe.spec(server_name="state_probe", tool_name="emit_marker",
                              marker="STORAGE_" + secrets.token_hex(8))
            agent = McpAgentWorkload(ROOT, self.output / "maas", binary.resolve(),
                probe=probe, spec=spec, model_tool_name="mcp__state_probe__emit_marker",
                tpot_ms=3, timeout_seconds=60)
            result["mcp_event_log"] = str(spec.event_log)
        else:
            agent = AgentWorkload(ROOT, self.output / "maas", binary.resolve(),
                kind="opencode", turns=2, input_bytes=128, tpot_ms=3,
                timeout_seconds=60, setup_timeout_seconds=300)
        try:
            environment.prepare()
            pid = int((environment.config.work_dir / "run/actraild.pid").read_text())
            if Path(f"/proc/{pid}/exe").resolve() != self.bins / "actraild":
                raise RuntimeError("owned daemon differs from selected release directory")
            daemon = DaemonCpu(pid)
            agent.start()
            task = self.output / "agent"
            agent.prepare(task)
            agent.reset()
            command = [str(self.bins / "actrailctl"), "--config",
                       str(environment.operator_config), "launch", "--", *agent.command(task)]
            (self.output / "command.json").write_text(json.dumps(command, indent=2) + "\n")
            with (task / "stdout.log").open("wb") as stdout, (task / "stderr.log").open("wb") as stderr:
                process = subprocess.Popen(command, cwd=task, env=dict(os.environ, **agent.env),
                                           stdout=stdout, stderr=stderr, start_new_session=True)
                try:
                    status = process.wait(timeout=60)
                finally:
                    if process.poll() is None:
                        os.killpg(process.pid, signal.SIGTERM)
                        try:
                            process.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            os.killpg(process.pid, signal.SIGKILL)
                            process.wait(timeout=5)
                if status:
                    raise RuntimeError(f"real agent exited {status}")
            result["workload"] = agent.validate(task, task / "stdout.log")
            result["online"] = self.wait_for_evidence(environment, daemon,
                                                       self.args.agent_kind == "xiaoo")
            environment.require_no_database()
            result.update(status="passed", main_database_created=environment.database.exists())
        except BaseException as error:
            result.update(status="failed", error=f"{type(error).__name__}: {error}")
            raise
        finally:
            agent.stop()
            try:
                environment.close()
                environment.require_no_database()
            except BaseException as error:
                result.update(status="failed", cleanup_error=f"{type(error).__name__}: {error}")
                raise
            finally:
                (self.output / "acceptance.json").write_text(json.dumps(result, indent=2) + "\n")

    @staticmethod
    def wait_for_evidence(environment, daemon, require_mcp):
        deadline = time.monotonic() + 30
        counts = {}
        while time.monotonic() < deadline:
            daemon.read_ms()
            log = (environment.config.work_dir / "log/actraild.log").read_text(errors="replace")
            traces = set(re.findall(r"agent_launch started trace_id=trace-(\d+)\b", log))
            finished = set(re.findall(r"trace_finalization completed trace_id=trace-(\d+)\b", log))
            latest = {}
            wire_traces = set()
            for span in environment.spans():
                attrs = environment.attributes(span)
                identity, kind = attrs.get("actrail.action.id"), attrs.get("actrail.action.kind")
                if identity and kind:
                    latest[(kind, identity)] = attrs
                    wire_traces.add(span["traceId"])
            counts = {kind: sum(key[0] == kind for key in latest)
                      for kind in {key[0] for key in latest}}
            complete = (counts.get("llm.request") == 2 and counts.get("llm.response") == 2
                        and (not require_mcp or all(counts.get(kind) == 1 for kind in MCP_KINDS)))
            successful = all(attrs.get("actrail.action.status") == "success"
                             for (kind, _), attrs in latest.items()
                             if kind in MCP_KINDS | {"llm.response"})
            if complete and successful and len(traces) == 1 and traces <= finished:
                if len(wire_traces) != 1:
                    raise RuntimeError(f"online actions have inconsistent OTLP trace IDs: {wire_traces}")
                return {"trace_id": int(next(iter(traces))), "trace_finalized": True,
                        "otlp_trace_id": next(iter(wire_traces)), "unique_actions": counts,
                        "llm_response_status": "success", "mcp_required": require_mcp}
            time.sleep(0.05)
        raise RuntimeError(f"online analysis/finalization incomplete: counts={counts}, "
                           f"started={traces}, finalized={finished}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--agent-kind", choices=("xiaoo", "opencode"), default="xiaoo")
    parser.add_argument("--agent-bin", type=Path)
    parser.add_argument("--config-patch", type=Path)
    parser.add_argument("--storage-backend", choices=("noop", "sqlite"), default="noop")
    StorageAcceptance(parser.parse_args()).run()
