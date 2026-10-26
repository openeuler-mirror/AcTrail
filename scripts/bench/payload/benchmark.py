"""Short, fixed-work comparisons across independent collection configurations."""

from __future__ import annotations

import argparse
import json
import math
import os
import shutil
import statistics
import subprocess
import time
import tomllib
from pathlib import Path

from scripts.bench.detail.benchmark import BenchmarkLock
from scripts.bench.overall.runtime import ReleaseBuild
from scripts.bench.payload.agent import AgentWorkload
from scripts.bench.payload.measurement import CommandMeasurement, DaemonProfile
from scripts.bench.payload.runtime import CollectionRuntime

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
WORKLOADS = ("fork", "exec", "read", "write", "agent", "idle", "stdio")


class PayloadBenchmark:
    def __init__(self, args: argparse.Namespace):
        self.args = args
        self.settings = tomllib.loads((HERE / "configs/benchmark.toml").read_text())
        self.settings.update(tomllib.loads((args.config_dir / "benchmark.toml").read_text()))
        for key in self.settings:
            override = getattr(args, key, None)
            if override is not None:
                self.settings[key] = override
        tpots = self.settings["agent_tpot_ms"]
        if not isinstance(tpots, list) or not tpots:
            raise ValueError("agent_tpot_ms must be a nonempty list of millisecond values")
        for key, value in self.settings.items():
            for number in value if key == "agent_tpot_ms" else [value]:
                if type(number) not in (int, float) or not math.isfinite(number):
                    raise ValueError(f"benchmark setting must be a finite number: {key}={number}")
                if number < 0 or (key not in ("warmups", "agent_tpot_ms") and number == 0):
                    raise ValueError(f"benchmark setting must be positive (warmups/TPOT may be zero): {key}={number}")
        if len(set(tpots)) != len(tpots):
            raise ValueError("agent_tpot_ms must not contain duplicate values")
        for key in ("rounds", "warmups", "fork_operations", "exec_operations", "task_iterations",
                    "read_operations", "write_operations", "block_bytes", "agent_turns",
                    "agent_input_bytes", "stdio_operations", "maas_max_request_bytes"):
            if type(self.settings[key]) is not int:
                raise ValueError(f"benchmark setting must be an integer: {key}")
        self.out = args.out.resolve()
        self.bin_dir = args.bin_dir.resolve()
        self.measurement = CommandMeasurement(self.settings["timeout_seconds"])
        self.driver = self.out / "workload"
        self.sleep_binary = shutil.which("sleep")
        self.agent_variants = {f"agent-tpot{str(tpot).removesuffix('.0')}": tpot for tpot in tpots}
        self.workloads = [name for kind in args.workloads
                          for name in (self.agent_variants if kind == "agent" else [kind])]
        self.agents: dict[str, AgentWorkload] = {}
        self.report: dict = {
            "status": "running", "settings": self.settings,
            "modes": args.modes, "workloads": self.workloads, "samples": [],
            "agent_kind": args.agent_kind,
            "diagnostic_perf": args.perf,
            "measurement": {
                "task_cpu": "wait4 user+system including reaped descendants; observed includes actrailctl launch",
                "daemon_cpu": "proc stat delta through trace finalization; startup and shutdown excluded",
                "cpu_overhead": "(task_cpu + daemon_cpu - bare_task_cpu) / bare_task_cpu",
                "wall": "command launch to exit; drain reported separately",
                "io": "page-cache reads/writes; no fsync or cache dropping",
                "server_cpu": "MaaS server and benchmark controller excluded",
            },
            "daemon_startup_ms": {},
        }

    def run(self) -> None:
        self.report['concurrent_daemons_at_start'] = CollectionRuntime.running_daemons()
        self._preflight()
        self.out.mkdir(parents=True, exist_ok=False)
        try:
            self._prepare()
            if "agent" in self.args.workloads:
                for name, tpot in self.agent_variants.items():
                    agent = AgentWorkload(
                        ROOT, self.out / "maas" / name, self.args.agent_bin,
                        turns=self.settings["agent_turns"], tpot_ms=tpot,
                        timeout_seconds=self.settings["timeout_seconds"],
                        kind=self.args.agent_kind,
                        setup_timeout_seconds=self.settings["build_timeout_seconds"],
                        input_bytes=self.settings["agent_input_bytes"],
                        max_request_bytes=self.settings["maas_max_request_bytes"],
                    )
                    self.agents[name] = agent
                    agent.start()
            for mode in self.args.modes:
                runtime = None
                try:
                    if mode != "0":
                        runtime = self._collection_runtime(mode)
                        started = time.monotonic()
                        runtime.start()
                        self.report["daemon_startup_ms"][mode] = (time.monotonic() - started) * 1000
                        shutil.copy2(runtime.config, self.out / "configs" / f"{mode}.resolved.toml")
                    for phase, rounds in (("warmup", self.settings["warmups"]),
                                          ("measured", self.settings["rounds"])):
                        for round_index in range(rounds):
                            for kind in self.workloads:
                                print(f"[{mode}/{kind}/{phase}] round {round_index + 1}", flush=True)
                                with DaemonProfile(runtime.cpu.pid if runtime and self.args.perf else None,
                                                   self.out / f"perf-{mode}-{kind}-{phase}-{round_index + 1}"):
                                    self._sample(mode, kind, round_index + 1, runtime, phase)
                                self._save()
                finally:
                    if runtime:
                        runtime.stop()
            self.report["status"] = "passed"
        except BaseException as error:
            self.report["status"] = "failed"
            self.report["error"] = f"{type(error).__name__}: {error}"
            raise
        finally:
            for agent in self.agents.values():
                agent.stop()
            self.report['concurrent_daemons_at_end'] = CollectionRuntime.running_daemons()
            self._save()
            if self.report["status"] == "passed" and not self.args.keep_runtime:
                for mode in self.args.modes:
                    directory = self.out / f"runtime-{mode}"
                    if directory.exists():
                        shutil.rmtree(directory)
                if (self.out / "maas").exists():
                    shutil.rmtree(self.out / "maas")
                self.driver.unlink(missing_ok=True)

    def _collection_runtime(self, mode: str) -> CollectionRuntime:
        return CollectionRuntime(
            self.out / f"runtime-{mode}", self.bin_dir,
            self.out / "configs" / f"{mode}.toml", self.settings,
        )

    def _preflight(self) -> None:
        if self.args.perf and shutil.which("perf") is None:
            raise RuntimeError("--perf requires the perf executable")
        if "idle" in self.args.workloads and self.sleep_binary is None:
            raise RuntimeError("idle workload requires the sleep executable")
        if os.geteuid() != 0 and any(mode != "0" for mode in self.args.modes):
            raise RuntimeError("P/C need root for the host collectors; run the benchmark with sudo")
        if shutil.which(self.args.cc) is None:
            raise RuntimeError(f"C compiler not found: {self.args.cc}")
        for mode in self.args.modes:
            if mode != "0":
                tomllib.loads((self.args.config_dir / f"{mode}.toml").read_text())
        if "agent" in self.args.workloads:
            if self.args.agent_kind == "opencode" and shutil.which("git") is None:
                raise RuntimeError("OpenCode workload isolation requires git")
            if self.args.agent_kind == "opencode" and shutil.which("npm") is None:
                raise RuntimeError("OpenCode workload preparation requires npm")
            if not self.args.agent_bin.is_file() or not os.access(self.args.agent_bin, os.X_OK):
                raise RuntimeError(f"real {self.args.agent_kind} executable missing: {self.args.agent_bin}; set --agent-bin")
            if shutil.which("openssl") is None:
                raise RuntimeError("local HTTPS MaaS requires openssl")

    def _prepare(self) -> None:
        snapshots = self.out / "configs"
        snapshots.mkdir()
        for name in ["benchmark.toml", *[f"{m}.toml" for m in self.args.modes if m != "0"]]:
            shutil.copy2(self.args.config_dir / name, snapshots / name)
        shutil.copy2(HERE / "maas/agent.json", snapshots / "agent.json")
        build = ReleaseBuild(ROOT)
        if self.args.skip_build:
            self.report["source_commit"] = build.commit_info()
            self.report["build"] = "skipped; binary provenance not verified against source commit"
        else:
            if self.bin_dir != ROOT / "target/release":
                raise ValueError("custom --bin-dir requires --skip-build")
            subprocess.run(["cargo", "fmt"], cwd=ROOT, check=True)
            print("building cargo --release", flush=True)
            self.report["source_commit"] = build.ensure(timeout_seconds=self.settings["build_timeout_seconds"])
            self.report["build"] = "cargo build --release"
        artifacts = {}
        names = ["actraild", "actrailctl"] if any(m != "0" for m in self.args.modes) else []
        for name in names:
            path = self.bin_dir / name
            if not path.is_file() or not os.access(path, os.X_OK):
                raise RuntimeError(f"release executable missing: {path}")
        for path in sorted(self.bin_dir.iterdir()):
            if path.is_file() and (path.name in names or path.suffix == ".so"):
                stat = path.stat()
                artifacts[path.name] = {"path": str(path), "bytes": stat.st_size, "mtime_ns": stat.st_mtime_ns}
        self.report["artifacts"] = artifacts
        self.report["host"] = {"uname": list(os.uname()), "clock_ticks": os.sysconf("SC_CLK_TCK")}
        self.report["agent_binary"] = str(self.args.agent_bin)
        subprocess.run(
            [self.args.cc, "-O2", "-std=c11", "-Wall", "-Wextra", "-Werror",
             str(HERE / "workload.c"), "-o", str(self.driver)], check=True, timeout=30,
        )

    def _command(self, kind: str, directory: Path) -> list[str]:
        if kind == "idle":
            return [self.sleep_binary, str(self.settings["idle_seconds"])]
        if kind in self.agents:
            agent = self.agents[kind]
            agent.prepare(directory)
            agent.reset()
            return agent.command(directory)
        count = self.settings[f"{kind}_operations"]
        command = [str(self.driver), kind, "--operations", str(count)]
        if kind == "exec":
            command += ["--task-iterations", str(self.settings["task_iterations"])]
        if kind in ("read", "write"):
            block = self.settings["block_bytes"]
            path = directory / "io.bin"
            # Materialize and warm both files outside measurement, under no trace.
            with path.open("wb") as stream:
                data = b"Z" * block
                for _ in range(count):
                    stream.write(data)
            command += ["--block-bytes", str(block), "--path", str(path)]
        return command

    def _validate(self, kind: str, directory: Path) -> dict:
        if kind == "idle":
            return {"command": self.sleep_binary, "sleep_seconds": self.settings["idle_seconds"]}
        if kind in self.agents:
            return self.agents[kind].validate(directory, directory / "stdout.log")
        lines = (directory / "stdout.log").read_text().splitlines()
        records = [json.loads(line) for line in lines if line.startswith('{"kind":')]
        if len(records) != 1:
            raise RuntimeError(f"missing unique workload completion record: {directory}")
        result = records[0]
        count = self.settings[f"{kind}_operations"]
        if result["kind"] != kind or result["operations"] != count:
            raise RuntimeError(f"workload operation count mismatch: {result}")
        if kind in ("read", "write"):
            expected_bytes = count * self.settings["block_bytes"]
            if result["bytes"] != expected_bytes or result["checksum"] != (180 * count) % (1 << 64):
                raise RuntimeError(f"I/O workload result mismatch: {result}")
            if (directory / "io.bin").stat().st_size != expected_bytes:
                raise RuntimeError("I/O file length mismatch")
        if kind == "stdio":
            if (lines.count("ZZZZZZZZZ") != count or result["bytes"] != count * 10
                    or result["io_calls"] < count):
                raise RuntimeError("stdio captured workload output mismatch")
        return result

    def _sample(self, mode: str, kind: str, round_index: int,
                runtime: CollectionRuntime | None, phase: str) -> None:
        directory = self.out / f"{mode}-{kind}-{phase}-{round_index}"
        directory.mkdir()
        sample = {"mode": mode, "workload": kind, "round": round_index,
                  "phase": phase, "status": "running"}
        agent = self.agents.get(kind)
        if agent:
            sample["agent_tpot_ms"] = agent.tpot_ms
        self.report["samples"].append(sample)
        try:
            command = self._command(kind, directory)
            env = dict(os.environ)
            if agent:
                env.update(agent.env)
            previous = runtime.mark() if runtime else 0
            cpu_start = runtime.cpu.read_ms() if runtime else 0
            result = self.measurement.run(runtime.launch(command) if runtime else command, directory, env)
            cpu_exit = runtime.cpu.read_ms() if runtime else 0
            sample.update(result)
            sample["daemon_run_cpu_ms"] = cpu_exit - cpu_start
            sample["drain_ms"] = 0.0
            if runtime:
                sample.update(runtime.drain(previous))
            cpu_end = runtime.cpu.read_ms() if runtime else 0
            sample["daemon_drain_cpu_ms"] = cpu_end - cpu_exit
            sample["daemon_cpu_ms"] = cpu_end - cpu_start
            sample["total_cpu_ms"] = result["task_cpu_ms"] + sample["daemon_cpu_ms"]
            sample["workload_result"] = self._validate(kind, directory)
            if runtime:
                sample["collection"] = runtime.evidence(previous, "agent" if agent else kind, directory)
            sample["status"] = "passed"
            self._print_sample(sample)
            (directory / "io.bin").unlink(missing_ok=True)
            for path in directory.glob("result-*.txt"):
                path.unlink()
            (directory / "input.txt").unlink(missing_ok=True)
            if agent:
                agent.cleanup(directory)
        except BaseException as error:
            sample["status"] = "failed"
            sample["error"] = f"{type(error).__name__}: {error}"
            raise

    def _print_sample(self, sample: dict) -> None:
        print(f"  wall={sample['wall_ms']:.1f}ms CPU task={sample['task_cpu_ms']:.1f}ms "
              f"daemon={sample['daemon_cpu_ms']:.1f}ms drain={sample['drain_ms']:.1f}ms", flush=True)

    def _save(self) -> None:
        summaries = []
        measured = [s for s in self.report["samples"]
                    if s["phase"] == "measured" and s["status"] == "passed"]
        for kind in self.workloads:
            baseline = [s for s in measured if s["mode"] == "0" and s["workload"] == kind]
            for mode in self.args.modes:
                samples = [s for s in measured if s["mode"] == mode and s["workload"] == kind]
                if not samples:
                    continue
                summary = {"mode": mode, "workload": kind, "rounds": len(samples)}
                for metric in ("wall_ms", "task_cpu_ms", "daemon_cpu_ms", "total_cpu_ms", "drain_ms"):
                    summary[metric] = statistics.mean(s[metric] for s in samples)
                if baseline:
                    bare_cpu = statistics.mean(s["total_cpu_ms"] for s in baseline)
                    bare_wall = statistics.mean(s["wall_ms"] for s in baseline)
                    summary["extra_cpu_ms"] = summary["total_cpu_ms"] - bare_cpu
                    summary["cpu_overhead_pct"] = 100 * (summary["total_cpu_ms"] / bare_cpu - 1) if bare_cpu else None
                    summary["wall_overhead_pct"] = 100 * (summary["wall_ms"] / bare_wall - 1)
                summaries.append(summary)
        self.report["summary"] = summaries
        (self.out / "results.json").write_text(json.dumps(self.report, indent=2, ensure_ascii=False) + "\n")
        rows = ["# Collection overhead matrix", "", f"Status: {self.report['status']}. "
                f"Agent: {self.args.agent_kind}. "
                f"Warmups per configuration/workload: {self.settings['warmups']} (excluded from statistics). "
                "Means of completed measured samples; one round is a quick measurement, not a significance claim.", "",
                "CPU includes the task process tree and daemon through trace finalization. "
                "Wall time excludes the separately reported drain. Values are milliseconds.", "",
                "Total CPU by workload and configuration (parentheses: extra CPU versus 0):", "",
                "| Workload | " + " | ".join(self.args.modes) + " |",
                "| --- | " + " | ".join("---:" for _ in self.args.modes) + " |"]
        if self.args.perf:
            rows[2:2] = ["Diagnostic perf sampling is enabled. These CPU values are excluded from performance comparisons.", ""]
        indexed = {(s["workload"], s["mode"]): s for s in summaries}
        for kind in self.workloads:
            cells = []
            for mode in self.args.modes:
                summary = indexed.get((kind, mode))
                if summary is None:
                    cells.append("—")
                    continue
                cell = f"{summary['total_cpu_ms']:.1f}"
                if mode != "0" and summary.get("cpu_overhead_pct") is not None:
                    cell += f" ({summary['cpu_overhead_pct']:+.1f}%)"
                cells.append(cell)
            rows.append("| " + kind + " | " + " | ".join(cells) + " |")
        rows += ["", "Detailed measurements:", "",
                "| Workload | Mode | Rounds | Task CPU | Daemon CPU | Total CPU | Extra CPU | CPU Δ | Wall | Wall Δ | Drain |",
                "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
        for s in summaries:
            cpu_pct = f"{s['cpu_overhead_pct']:+.1f}%" if s.get("cpu_overhead_pct") is not None else "—"
            wall_pct = f"{s['wall_overhead_pct']:+.1f}%" if "wall_overhead_pct" in s else "—"
            extra = f"{s['extra_cpu_ms']:.1f}" if "extra_cpu_ms" in s else "—"
            rows.append(f"| {s['workload']} | {s['mode']} | {s['rounds']} | {s['task_cpu_ms']:.1f} "
                        f"| {s['daemon_cpu_ms']:.1f} | {s['total_cpu_ms']:.1f} | {extra} | {cpu_pct} "
                        f"| {s['wall_ms']:.1f} | {wall_pct} | {s['drain_ms']:.1f} |")
        if self.report.get("error"):
            rows += ["", f"Failure: {self.report['error']}"]
        (self.out / "results.md").write_text("\n".join(rows) + "\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config-dir", type=Path, default=HERE / "configs")
    parser.add_argument("--out", type=Path, default=ROOT / "local/bench/payload" / time.strftime("%Y%m%d-%H%M%S"))
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/release")
    parser.add_argument("--agent-kind", choices=("xiaoo", "opencode"), default="xiaoo")
    parser.add_argument("--agent-bin", type=Path)
    parser.add_argument("--cc", default="cc")
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--keep-runtime", action="store_true", help="retain runtime DB/logs and local MaaS files")
    parser.add_argument("--modes", nargs="+", choices=("0", "P", "C"), default=["0", "P", "C"])
    parser.add_argument("--workloads", nargs="+", choices=WORKLOADS, default=list(WORKLOADS[:-1]))
    parser.add_argument("--perf", action="store_true", help="diagnostic daemon sampling; exclude from CPU comparisons")
    parser.add_argument("--lock-path", type=Path, default=Path("/run/lock/actrail-v2-regression.lock"))
    for option in ("rounds", "warmups", "fork-operations", "exec-operations", "task-iterations",
                   "read-operations", "write-operations", "block-bytes", "agent-turns",
                   "agent-input-bytes", "stdio-operations", "maas-max-request-bytes"):
        parser.add_argument(f"--{option}", type=int)
    for option in ("timeout-seconds", "drain-timeout-seconds", "idle-seconds"):
        parser.add_argument(f"--{option}", type=float)
    parser.add_argument("--agent-tpot-ms", nargs="+", type=float,
                        help="MaaS TPOT values in milliseconds; each value gets its own agent row and bare baseline")
    args = parser.parse_args()
    if args.agent_bin is None:
        args.agent_bin = Path(shutil.which(args.agent_kind) or args.agent_kind)
    if len(set(args.modes)) != len(args.modes) or len(set(args.workloads)) != len(args.workloads):
        parser.error("modes and workloads must not contain duplicates")
    try:
        benchmark = PayloadBenchmark(args)
        with BenchmarkLock(args.lock_path, benchmark.settings["lock_timeout_seconds"]):
            benchmark.run()
        print(f"results: {benchmark.out / 'results.md'}", flush=True)
        return 0
    except KeyboardInterrupt:
        return 130
    except Exception as error:
        print(f"benchmark failed: {error}", flush=True)
        return 1
