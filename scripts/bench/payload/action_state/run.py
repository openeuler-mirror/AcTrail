"""Exercise action lifecycle persistence through real xiaoo and local MCP."""
import argparse
import json
import os
import secrets
import shutil
from pathlib import Path

from scripts.bench.overall.runtime.config_patch import ConfigPatch
from scripts.bench.payload.benchmark import ROOT
from scripts.bench.payload.measurement import CommandMeasurement, DaemonCpu
from scripts.bench.payload.runtime import CollectionRuntime
from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core import TestOutput
from tests.v2.common.mcp_test_support.probe import McpProbeWorkspace

from .workload import McpAgentWorkload


class ActionStateAcceptance:
    def __init__(self, args):
        self.args = args
        self.out = args.out.resolve()
        self.bins = args.bin_dir.resolve()

    def run(self):
        self.out.mkdir(parents=True, exist_ok=False)
        report = {"status": "running", "scope": "functional only", "samples": []}
        try:
            for mode in self.args.modes:
                report["samples"].append(self.scenario(mode))
            report["status"] = "passed"
        except BaseException as error:
            report.update(status="failed", error=f"{type(error).__name__}: {error}")
            raise
        finally:
            (self.out / "acceptance.json").write_text(json.dumps(report, indent=2) + "\n")

    def scenario(self, mode):
        directory = self.out / mode
        directory.mkdir()
        probe = McpProbeWorkspace(ROOT, directory, "mcp")
        spec = probe.spec(server_name="state_probe", tool_name="emit_marker",
                          marker="STATE_" + secrets.token_hex(8))
        agent = McpAgentWorkload(ROOT, directory / "maas", self.args.agent_bin,
            probe=probe, spec=spec, model_tool_name=self.args.model_tool_name,
            tpot_ms=3, timeout_seconds=60)
        runtime = None
        collection = None
        context_probe = None
        result = {"mode": mode, "probe_events": str(spec.event_log), "status": "running"}
        try:
            if mode != "bare":
                work = self.out / f"runtime-{mode}"
                work.mkdir()
                patch = work / "actraild.patch.toml"
                ActrailRuntime.write_isolated_operator_config_patch(patch, work)
                ConfigPatch(self.args.config_dir / f"{mode}.toml").apply_isolation(patch)
                runtime = ActrailRuntime(ROOT, self.bins, 60, TestOutput(), work / "actraild.conf", patch)
                runtime.prepare()
                collection = CollectionRuntime(work, self.bins, patch,
                    {"agent_turns": 2, "drain_timeout_seconds": 30, "poll_seconds": 0.01})
                pid = int((work / "run/actraild.pid").read_text())
                if Path(f"/proc/{pid}/exe").resolve() != self.bins / "actraild":
                    raise RuntimeError("unexpected isolated daemon executable")
                collection.cpu = DaemonCpu(pid)
                collection.log = (work / "log/actraild.log").open(errors="replace")
                result["resolved_config"] = str(work / "actraild.conf")
                if self.args.context_probe_event:
                    from .context_probe import ContextProbe
                    context_probe = ContextProbe(self.args.context_probe_event, pid, directory)
            agent.start()
            task = directory / "agent"
            agent.prepare(task)
            agent.reset()
            command = agent.command(task)
            previous = collection.mark() if collection else None
            if collection:
                command = collection.launch(command)
            (directory / "command.json").write_text(json.dumps(command, indent=2))
            CommandMeasurement(60).run(command, task, dict(os.environ, **agent.env))
            result["workload"] = agent.validate(task, task / "stdout.log")
            if collection:
                result["finalization"] = collection.drain(previous)
                result["collection"] = collection.evidence(previous, "agent", task)
                from .verify import ActionStateVerifier
                trace = result["collection"]["traces"][0][0]
                verifier = ActionStateVerifier(self.bins, work, directory, spec)
                result["readback"] = (verifier.verify(trace) if self.args.expect_mcp == "present"
                                      else verifier.verify_disabled(trace))
            result["status"] = "passed"
            return result
        except BaseException as error:
            result.update(status="failed", error=f"{type(error).__name__}: {error}")
            raise
        finally:
            probe_error = None
            if context_probe:
                try:
                    result["context_uploads"] = context_probe.stop()
                except BaseException as error:
                    probe_error = error
                    result.update(status="failed", error=f"context probe: {error}")
            agent.stop()
            if collection and collection.log:
                collection.log.close()
            if runtime:
                stopped = runtime.stop()
                if stopped is not None and stopped.returncode:
                    raise RuntimeError("own isolated daemon failed to stop")
            (directory / "result.json").write_text(json.dumps(result, indent=2) + "\n")
            if probe_error:
                raise probe_error


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/release")
    parser.add_argument("--config-dir", type=Path, default=Path(__file__).resolve().parents[1] / "configs")
    parser.add_argument("--agent-bin", type=Path, default=Path(shutil.which("xiaoo") or "xiaoo"))
    parser.add_argument("--model-tool-name", default="mcp__state_probe__emit_marker")
    parser.add_argument("--modes", nargs="+", choices=("bare", "P", "C"), default=["P", "C"])
    parser.add_argument("--expect-mcp", choices=("present", "absent"), default="present")
    parser.add_argument("--context-probe-event", help="Existing perf uprobe event with a u32 aux field; functional runs only")
    ActionStateAcceptance(parser.parse_args()).run()
