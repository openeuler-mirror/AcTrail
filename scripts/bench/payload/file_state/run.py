"""Verify file observations from a real xiaoo tool and actual FD syscalls."""
import argparse
import json
import os
import shlex
import subprocess
from pathlib import Path

from scripts.bench.overall.runtime.config_patch import ConfigPatch
from scripts.bench.payload.agent import AgentWorkload
from scripts.bench.payload.benchmark import ROOT
from scripts.bench.payload.measurement import CommandMeasurement, DaemonCpu
from scripts.bench.payload.runtime import CollectionRuntime
from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core import TestOutput

from .verify import FileStateVerifier


class FileAgentWorkload(AgentWorkload):
    def __init__(self, *args, helper, **kwargs):
        super().__init__(*args, turns=2, input_bytes=128, tpot_ms=3, **kwargs)
        self._fixture["tool_command"] = shlex.quote(str(helper)) + " && cat input.txt > result-{index}.txt"


class FileStateAcceptance:
    def __init__(self, args):
        self.args = args

    def run(self):
        directory = self.args.out.resolve()
        directory.mkdir(parents=True, exist_ok=False)
        bins = (ROOT / "target/release").resolve()
        helper = directory / "file-helper"
        source = "bulk_workload.c" if self.args.bulk_read_retention else "workload.c"
        subprocess.run(["cc", "-O2", "-Wall", "-Wextra", str(Path(__file__).with_name(source)),
                        "-o", str(helper)], check=True)
        work = directory / "runtime"
        work.mkdir()
        patch = work / "patch.toml"
        ActrailRuntime.write_isolated_operator_config_patch(patch, work)
        ConfigPatch(self.args.config).apply_isolation(patch)
        runtime = ActrailRuntime(ROOT, bins, 60, TestOutput(), work / "actraild.conf", patch)
        agent = FileAgentWorkload(ROOT, directory / "maas", self.args.agent_bin,
                                  helper=helper, timeout_seconds=60)
        collection = None
        context_probe = None
        holder = None
        result = {"status": "running", "scope": "real-agent functional file verification"}
        try:
            runtime.prepare()
            collection = CollectionRuntime(work, bins, patch,
                {"agent_turns": 2, "drain_timeout_seconds": 30, "poll_seconds": 0.01})
            pid = int((work / "run/actraild.pid").read_text())
            if Path(f"/proc/{pid}/exe").resolve() != bins / "actraild":
                raise RuntimeError("unexpected daemon executable")
            collection.cpu = DaemonCpu(pid)
            collection.log = (work / "log/actraild.log").open(errors="replace")
            if self.args.hold_file_trace:
                from .held_trace import HeldFileTrace
                holder = HeldFileTrace(collection, pid, self.args.context_probe_event, directory / "holder")
                holder.start()
            if self.args.context_probe_event:
                from scripts.bench.payload.action_state.context_probe import ContextProbe
                context_probe = ContextProbe(self.args.context_probe_event, pid, directory)
            agent.start()
            task = directory / "agent"
            agent.prepare(task)
            agent.reset()
            previous = collection.mark()
            command = collection.launch(agent.command(task))
            if self.args.host_ebpf:
                command[4:4] = ["--host-ebpf", self.args.host_ebpf, "--seccomp-notify", "disabled"]
            (directory / "command.json").write_text(json.dumps(command, indent=2))
            CommandMeasurement(60).run(command, task, dict(os.environ, **agent.env))
            result["workload"] = agent.validate(task, task / "stdout.log")
            result["finalization"] = collection.drain(previous)
            result["collection"] = collection.evidence(previous, "agent", task,
                require_llm_capture=holder is None)
            trace = result["collection"]["traces"][0][0]
            if self.args.bulk_read_retention:
                from .bulk_verify import BulkReadVerifier
                result["readback"] = BulkReadVerifier(bins, work, directory, helper,
                    self.args.bulk_read_retention).verify(trace)
            else:
                result["readback"] = FileStateVerifier(bins, work, directory, helper,
                    mmap_only=self.args.mmap_only, no_files=self.args.no_file_observations).verify(trace)
            result["status"] = "passed"
        except BaseException as error:
            result.update(status="failed", error=f"{type(error).__name__}: {error}")
            raise
        finally:
            probe_error = None
            if context_probe:
                try:
                    result["context_uploads"] = context_probe.stop()
                    if holder:
                        result["mixed_trace"] = holder.verify(result["context_uploads"])
                    else:
                        FileStateVerifier.verify_uploads(result["context_uploads"])
                except BaseException as error:
                    probe_error = error
                    result.update(status="failed", error=f"context probe: {error}")
            try:
                agent.stop()
            finally:
                try:
                    try:
                        if holder:
                            holder.stop()
                    finally:
                        if collection and collection.log:
                            collection.log.close()
                        stopped = runtime.stop()
                        if stopped is not None and stopped.returncode:
                            raise RuntimeError("isolated daemon failed to stop")
                finally:
                    (directory / "acceptance.json").write_text(json.dumps(result, indent=2) + "\n")
            if probe_error:
                raise probe_error


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--agent-bin", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--config", type=Path, default=ROOT / "scripts/bench/payload/configs/C.toml")
    parser.add_argument("--context-probe-event")
    expectations = parser.add_mutually_exclusive_group()
    expectations.add_argument("--mmap-only", action="store_true", help="Expect only configured mmap file observations")
    expectations.add_argument("--no-file-observations", action="store_true")
    expectations.add_argument("--bulk-read-retention", choices=("full", "errors_only"))
    parser.add_argument("--host-ebpf", choices=("enabled", "disabled"))
    parser.add_argument("--hold-file-trace", action="store_true")
    args = parser.parse_args()
    if args.hold_file_trace and not (args.context_probe_event and args.no_file_observations
                                    and args.host_ebpf == "disabled"):
        parser.error("--hold-file-trace requires a context probe, --no-file-observations and --host-ebpf disabled")
    FileStateAcceptance(args).run()
