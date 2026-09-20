"""Serial, fixed-work CPU comparison with explicitly selected frozen binaries."""

import argparse
import json
import shutil
import statistics
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "tests/v2/common/test_suites/local_maas_server"))

from scripts.bench.detail.benchmark import BenchmarkLock
from scripts.bench.payload.benchmark import PayloadBenchmark
from scripts.bench.payload.storage_cpu.runtime import StorageCpuRuntime


class StorageCpuBenchmark(PayloadBenchmark):
    def _preflight(self):
        self.binary_commit = (self.bin_dir / "commit.txt").read_text().strip()
        if self.binary_commit != self.args.expected_commit:
            raise RuntimeError("frozen commit.txt does not match --expected-commit")
        super()._preflight()

    def _prepare(self):
        super()._prepare()
        self.report["runner_commit"] = self.report.pop("source_commit")
        self.report.update(binary_commit=self.binary_commit,
                           binary_directory=str(self.bin_dir), storage_backend=self.args.backend,
                           provenance="commit.txt and fixed artifact paths; no binary hash verification")
        self.report["measurement"]["daemon_cpu"] = (
            "proc stat from launch through matching log finalization; identical SQLite/NoOp window; "
            "database evidence queries after CPU sampling")
        for mode in (mode for mode in self.args.modes if mode != "0"):
            patch = self.out / "configs" / f"{mode}.toml"
            with patch.open("a") as stream:
                stream.write(f'\n[storage]\nbackend = "{self.args.backend}"\n')

    def _collection_runtime(self, mode):
        return StorageCpuRuntime(self.out / f"runtime-{mode}", self.bin_dir,
                                 self.out / "configs" / f"{mode}.toml", self.settings,
                                 self.args.backend)

    def _save(self):
        summaries = []
        measured = [sample for sample in self.report["samples"]
                    if sample["phase"] == "measured" and sample["status"] == "passed"]
        for workload in self.workloads:
            bare = [s for s in measured if s["workload"] == workload and s["mode"] == "0"]
            for mode in self.args.modes:
                samples = [s for s in measured if s["workload"] == workload and s["mode"] == mode]
                if not samples:
                    continue
                summary = {"mode": mode, "workload": workload, "rounds": len(samples)}
                for metric in ("wall_ms", "task_cpu_ms", "daemon_cpu_ms", "total_cpu_ms", "drain_ms"):
                    summary[metric] = statistics.mean(s[metric] for s in samples)
                if bare:
                    baseline = statistics.mean(s["task_cpu_ms"] for s in bare)
                    summary["bare_task_cpu_ms"] = baseline
                    summary["extra_cpu_ms"] = summary["total_cpu_ms"] - baseline
                    summary["cpu_overhead_pct"] = 100 * summary["extra_cpu_ms"] / baseline
                summaries.append(summary)
        self.report["summary"] = summaries
        (self.out / "results.json").write_text(json.dumps(self.report, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--expected-commit", required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--backend", choices=("sqlite", "noop"), required=True)
    parser.add_argument("--config-dir", type=Path, default=Path(__file__).parent / "configs")
    parser.add_argument("--agent-kind", choices=("xiaoo", "opencode"), default="xiaoo")
    parser.add_argument("--agent-bin", type=Path)
    parser.add_argument("--modes", nargs="+", choices=("0", "P", "C"), default=["0", "P", "C"])
    parser.add_argument("--rounds", type=int)
    parser.add_argument("--warmups", type=int)
    parser.add_argument("--agent-turns", type=int)
    parser.add_argument("--agent-input-bytes", type=int)
    parser.add_argument("--agent-tpot-ms", nargs="+", type=float)
    parser.add_argument("--timeout-seconds", type=float)
    parser.add_argument("--drain-timeout-seconds", type=float)
    parser.add_argument("--cc", default="cc")
    parser.add_argument("--lock-path", type=Path, default=Path("/run/lock/actrail-v2-regression.lock"))
    args = parser.parse_args()
    if len(set(args.modes)) != len(args.modes):
        parser.error("modes must not contain duplicates")
    args.workloads = ["agent"]
    if args.agent_bin is None:
        args.agent_bin = Path(shutil.which(args.agent_kind) or args.agent_kind)
    args.skip_build, args.keep_runtime, args.perf = True, True, False
    benchmark = StorageCpuBenchmark(args)
    with BenchmarkLock(args.lock_path, benchmark.settings["lock_timeout_seconds"]):
        benchmark.run()
    print(f"results: {benchmark.out / 'results.json'}", flush=True)


if __name__ == "__main__":
    main()
