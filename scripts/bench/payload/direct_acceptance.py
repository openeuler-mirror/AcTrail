"""Real-agent TLS direct-copy functional acceptance with an isolated daemon.

P applies configs/tls-bpf-copy.toml over its normal profile. C uses its normal
profile. This is not a CPU benchmark. Other daemon instances are left untouched.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import threading
import time
import tomllib
from pathlib import Path

from scripts.bench.overall.runtime.config_patch import ConfigPatch
from scripts.bench.payload.agent import AgentWorkload
from scripts.bench.payload.benchmark import ROOT
from scripts.bench.payload.measurement import CommandMeasurement, DaemonCpu
from scripts.bench.payload.profile_acceptance import ProfileAcceptance
from scripts.bench.payload.limited_acceptance import LimitedAcceptance
from scripts.bench.payload.runtime import CollectionRuntime
from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core import TestOutput


class AgentObserver:
    def __init__(self, executable):
        self.executable = executable.resolve()
        self.rows = {}
        self.stop = threading.Event()
        self.worker = threading.Thread(target=self.run, daemon=True)

    def run(self):
        while not self.stop.is_set():
            for process in Path("/proc").iterdir():
                if not process.name.isdigit():
                    continue
                try:
                    if (process / "exe").resolve(strict=True) != self.executable:
                        continue
                    stat = (process / "stat").read_text().rsplit(")", 1)[1].split()
                    key = (int(process.name), stat[19])
                    runtime = [line for line in (process / "maps").read_text().splitlines()
                               if "libactrail_tls_payload_probe_sync" in line]
                    row = self.rows.setdefault(key, dict(pid=key[0], start_ticks=key[1], observations=0,
                                                        runtime_mappings=[]))
                    row["observations"] += 1
                    row["runtime_mappings"] = sorted(set(row["runtime_mappings"] + runtime))
                except OSError:
                    continue
            self.stop.wait(0.02)

    def finish(self):
        self.stop.set()
        self.worker.join(timeout=5)
        return list(self.rows.values())


class DirectAcceptance:
    def __init__(self, args, mode):
        self.args = args
        self.mode = mode
        self.out = args.out.resolve()
        self.bin_dir = args.bin_dir.resolve()
        self.report = dict(status="running", scope="functional only; concurrent host daemons allowed; no CPU comparison", samples=[],
                           settings=dict(turns=args.turns, input_bytes=args.input_bytes,
                                         limited_acceptance=args.limited_acceptance,
                                         via_bash_exec=args.via_bash_exec))

    def run(self):
        work = self.out / f"runtime-{self.mode}"
        work.mkdir()
        config, patch = work / "actraild.conf", work / "actraild.patch.toml"
        ActrailRuntime.write_isolated_operator_config_patch(patch, work)
        profile = ConfigPatch(self.args.config_dir / f"{self.mode}.toml")
        if self.mode == "P":
            overlay = tomllib.loads((Path(__file__).parent / "configs/tls-bpf-copy.toml").read_text())
            profile.values.setdefault("payload", {}).setdefault("tls", {}).update(overlay["payload"]["tls"])
            self.report["tls_override"] = overlay["payload"]["tls"]
        profile.apply_isolation(patch)
        runtime = ActrailRuntime(ROOT, self.bin_dir, 60, TestOutput(), config, patch)
        observer = AgentObserver(self.args.agent_bin)
        settings = dict(agent_turns=self.args.turns, drain_timeout_seconds=30, poll_seconds=0.01)
        collection = CollectionRuntime(work, self.bin_dir, patch, settings)
        log_stream = None
        self.report["artifacts"] = {}
        for path in [self.bin_dir / name for name in ("actraild", "actrailctl", "actrailviewer") ] + [self.args.agent_bin]:
            with path.open("rb") as source:
                self.report["artifacts"][str(path)] = hashlib.file_digest(source, "sha256").hexdigest()
        try:
            runtime.prepare()
            pid = int((work / "run/actraild.pid").read_text())
            executable = (Path("/proc") / str(pid) / "exe").resolve(strict=True)
            if executable != (self.bin_dir / "actraild").resolve():
                raise RuntimeError(f"unexpected daemon executable: {executable}")
            self.report["daemon"] = dict(pid=pid, executable=str(executable), config=str(config))
            actual = tomllib.loads(config.read_text())
            if self.mode == "P":
                if (actual["payload"]["tls"]["capture_backend"] != "bpf-copy"
                        or actual["seccomp_notify"]["enabled"] or actual["process_seccomp"]["enabled"]):
                    raise RuntimeError("effective direct-copy configuration mismatch")
            elif actual["payload"]["tls"]["capture_backend"] != "tls-sync":
                raise RuntimeError("C must retain tls-sync")
            snapshots = self.out / "configs"
            snapshots.mkdir(exist_ok=True)
            shutil.copy2(config, snapshots / f"{self.mode}.resolved.toml")
            collection.cpu = DaemonCpu(pid)
            log_stream = (work / "log/actraild.log").open(errors="replace")
            collection.log = log_stream
            observer.worker.start()
            for tpot in (0, 3):
                agent = AgentWorkload(ROOT, self.out / f"maas-{self.mode}-{tpot}", self.args.agent_bin,
                                      turns=self.args.turns, input_bytes=self.args.input_bytes,
                                      tpot_ms=tpot, timeout_seconds=60)
                try:
                    agent.start()
                    directory = self.out / f"agent-{self.mode}-{tpot}"
                    agent.prepare(directory)
                    agent.reset()
                    previous = collection.mark()
                    env = dict(os.environ)
                    env.update(agent.env)
                    env["ACTRAIL_LAUNCH_TIMING"] = "1"
                    target_command = agent.command(directory)
                    if self.args.via_bash_exec:
                        target_command = ["/bin/bash", "-c", 'exec "$@"', "actrail-dynamic", *target_command]
                    launch_command = collection.launch(target_command)
                    (directory / "launch-command.json").write_text(json.dumps(launch_command, indent=2) + "\n")
                    CommandMeasurement(60).run(launch_command, directory, env)
                    sample = dict(mode=self.mode, tpot_ms=tpot, workload=agent.validate(directory, directory / "stdout.log"))
                    sample["launch_command"] = launch_command
                    self.report["samples"].append(sample)
                    sample["finalization"] = collection.drain(previous)
                    sample["collection"] = collection.evidence(previous, "agent", directory)
                    trace_id = sample["collection"]["traces"][0][0]
                    root = collection.query(
                        "SELECT p.host_pid,p.host_start_ticks FROM traces t JOIN processes p "
                        "ON p.process_id=t.root_process_id WHERE t.trace_id=?", (trace_id,))
                    if len(root) != 1 or root[0][0] is None or root[0][1] is None:
                        raise RuntimeError("trace root host identity is unresolved")
                    sample["root_host_identity"] = dict(pid=root[0][0], start_ticks=str(root[0][1]))
                    stderr = (directory / "stderr.log").read_text()
                    if self.mode == "P" and ("seccomp_enabled=true" in stderr or "payload_tls_seccomp=true" in stderr):
                        raise RuntimeError("launch unexpectedly enabled seccomp")
                    if self.mode == "P" and "seccomp_enabled=false" not in stderr:
                        raise RuntimeError("launch timing did not establish disabled seccomp")
                    validator = LimitedAcceptance if self.args.limited_acceptance else ProfileAcceptance
                    sample["profile"] = validator(self.out).trace(sample, self.args.turns)
                    sample["status"] = "passed"
                finally:
                    agent.stop()
            wanted = {(s["root_host_identity"]["pid"], s["root_host_identity"]["start_ticks"])
                      for s in self.report["samples"]}
            rows = [row for row in observer.finish() if (row["pid"], row["start_ticks"]) in wanted]
            self.report["agent_maps"] = rows
            if {(row["pid"], row["start_ticks"]) for row in rows} != wanted:
                raise RuntimeError("one or more trace root agents were not observed")
            if self.mode == "P" and any(row["runtime_mappings"] for row in rows):
                raise RuntimeError("P TLS runtime injection was found")
            if self.mode == "C" and not any(row["runtime_mappings"] for row in rows):
                raise RuntimeError("C TLS runtime injection was not observed")
            self.report["status"] = "passed"
        except BaseException as error:
            self.report.update(status="failed", error=f"{type(error).__name__}: {error}")
            raise
        finally:
            if observer.worker.ident:
                rows = observer.finish()
                wanted = {(s["root_host_identity"]["pid"], s["root_host_identity"]["start_ticks"])
                          for s in self.report["samples"] if "root_host_identity" in s}
                self.report["agent_maps"] = [row for row in rows if (row["pid"], row["start_ticks"]) in wanted]
            if log_stream:
                log_stream.close()
            try:
                stopped = runtime.stop()
                if stopped is not None and stopped.returncode:
                    raise RuntimeError("isolated daemon stop failed")
            finally:
                (self.out / f"acceptance-{self.mode}.json").write_text(json.dumps(self.report, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--config-dir", type=Path, default=Path(__file__).parent / "configs")
    parser.add_argument("--modes", choices=("P", "C"), nargs="+", default=["P", "C"])
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/release")
    parser.add_argument("--agent-bin", type=Path, default=Path("/home/yzh/.cargo/bin/xiaoo"))
    parser.add_argument("--turns", type=int, default=4)
    parser.add_argument("--input-bytes", type=int, default=1024)
    parser.add_argument("--limited-acceptance", action="store_true")
    parser.add_argument("--via-bash-exec", action="store_true")
    args = parser.parse_args()
    if args.turns < 1 or args.input_bytes < 1 or (args.limited_acceptance and args.modes != ["P"]):
        parser.error("positive workload sizes are required; limited acceptance requires --modes P")
    args.out.resolve().mkdir(parents=True, exist_ok=False)
    for mode in args.modes:
        DirectAcceptance(args, mode).run()
