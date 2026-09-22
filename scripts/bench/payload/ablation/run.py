"""One-command serial CPU ablation with live Markdown reports."""

import argparse
import contextlib
import json
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
import traceback
from datetime import datetime
from pathlib import Path

from scripts.bench.detail.benchmark import BenchmarkLock
from scripts.bench.overall.runtime.config_patch import ConfigPatch
from scripts.bench.payload.benchmark import PayloadBenchmark
from scripts.bench.payload.storage_cpu.run import StorageCpuBenchmark
from scripts.bench.payload.storage_cpu.runtime import StorageCpuRuntime


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]


class ProgressLog:
    def __init__(self, terminal, log):
        self.terminal, self.log = terminal, log

    def write(self, value):
        self.terminal.write(value)
        self.log.write(value)
        self.flush()
        return len(value)

    def flush(self):
        self.terminal.flush()
        self.log.flush()


class AblationReport:
    def __init__(self, output):
        self.output = output

    def write(self, run):
        settings = run["settings"]
        backends = run.get("storage_backends", {})
        storage = ", ".join(sorted(set(backends.values()))) if backends else run.get("storage_backend", "noop")
        lines = ["# CPU ablation", "",
                 "| Status | Agent binary | Storage | Warmups | Rounds | Requests/run | Input bytes | TPOT ms |",
                 "|---|---|---|---:|---:|---:|---:|---|",
                 f"| {run['status']} | {run.get('agent_binary', run['agent_kind'])} | {storage} | {settings['warmups']} | "
                 f"{settings['rounds']} | {settings['agent_turns']} | {settings['agent_input_bytes']} | "
                 f"{settings['agent_tpot_ms']} |",
                 "", "| Case | Workload | n | Bare CPU ms | Task + daemon CPU ms | Extra CPU ms | Extra CPU / bare |",
                 "|---|---|---:|---:|---:|---:|---:|"]
        bare = {s["workload"]: s["task_cpu_ms"] for s in run.get("summary", []) if s["mode"] == "0"}
        for row in run.get("summary", []):
            baseline = bare.get(row["workload"])
            if not baseline:
                continue
            total = row["total_cpu_ms"]
            extra = total - baseline
            lines.append(f"| {row['mode']} | {row['workload']} | {row['rounds']} | {baseline:.1f} | "
                         f"{total:.1f} | "
                         f"{extra:+.1f} | {extra / baseline * 100:+.1f}% |")
        lines += ["", "CPU covers command launch through trace finalization. Daemon startup and shutdown are excluded. "
                  "SQLite transactions are committed within this window; a final WAL checkpoint is not measured.",
                  "", "| Case | Workload | Storage | Task CPU ms | Daemon CPU ms | Command wall ms | Drain ms |",
                  "|---|---|---|---:|---:|---:|---:|"]
        for row in run.get("summary", []):
            backend = "—" if row["mode"] == "0" else backends.get(row["mode"], "noop")
            lines.append(f"| {row['mode']} | {row['workload']} | {backend} | {row['task_cpu_ms']:.1f} | "
                         f"{row['daemon_cpu_ms']:.1f} | {row['wall_ms']:.1f} | {row['drain_ms']:.1f} |")
        indexed = {(s["mode"], s["workload"]): s for s in run.get("summary", [])}
        lines += ["",
                  "| Stage | Parent | Workload | Total − parent (ms) | Extra − parent (pp of bare) |",
                  "|---|---|---|---:|---:|"]
        for case in run.get("case_metadata", []):
            for workload, baseline in bare.items():
                row = indexed.get((case["name"], workload))
                parent = indexed.get((case.get("parent"), workload))
                if row is None or parent is None or not baseline:
                    continue
                total = row["total_cpu_ms"] - parent["total_cpu_ms"]
                lines.append(f"| {case['name']} | {case['parent']} | {workload} | "
                             f"{total:+.1f} | {total / baseline * 100:+.1f} |")
        lines += ["", "| Stage | Parent | Change / status | Configuration |",
                  "|---|---|---|---|"]
        for case in run.get("case_metadata", []):
            state = f"TODO: {case['blocked']}" if case.get("blocked") else case.get("description", "")
            name = case["name"]
            config = "—" if case.get("blocked") else f"[TOML](measurements/configs/{name}.resolved.toml)"
            lines.append(f"| {name} | {case.get('parent', '—')} | {state} | {config} |")
        lines += ["", "| Artifact | Path |", "|---|---|",
                  "| Raw CPU | [results.json](measurements/results.json) |",
                  "| Progress | [run.log](run.log) |"]
        if run.get("error"):
            lines += ["", "```text", run["error"], "```"]
        document = "\n".join(lines) + "\n"
        (self.output / "report.md").write_text(document)
        return document


class AblationBenchmark(StorageCpuBenchmark):
    def _print_sample(self, sample):
        print(f"  CPU total={sample['total_cpu_ms']:.1f}ms", flush=True)

    def run(self):
        # The alias keeps Unix socket addresses short while artifacts live in --out.
        directory = tempfile.mkdtemp(prefix="actrail-a-", dir="/tmp")
        try:
            self.runtime_root = Path(directory) / "r"
            self.runtime_root.symlink_to(self.out, target_is_directory=True)
            super().run()
        except BaseException:
            # Keep configuration/socket access if daemon shutdown itself failed.
            print(f"Run failed; runtime alias retained for recovery: {directory}", file=sys.stderr)
            raise
        else:
            shutil.rmtree(directory)

    def _collection_runtime(self, mode):
        work = self.runtime_root / f"runtime-{mode}"
        if len(str(work / "run/tls-sync.sock").encode()) >= 108:
            raise ValueError(f"case name is too long for a Unix socket address: {mode}")
        return StorageCpuRuntime(work, self.bin_dir, self.out / "configs" / f"{mode}.toml",
                                 self.settings, self.args.storage_backends[mode])

    def _preflight(self):
        PayloadBenchmark._preflight(self)

    def _prepare(self):
        PayloadBenchmark._prepare(self)
        self.report["runner_commit"] = self.report.pop("source_commit")
        self.report["binary_commit"] = None
        self.report["storage_backends"] = self.args.storage_backends
        self.report["measurement"]["daemon_cpu"] = (
            "proc stat from command launch through matching log finalization for every backend; "
            "startup/shutdown excluded; evidence queries after sampling")
        self.report["case_sources"] = self.args.case_sources
        self.report["case_metadata"] = self.args.case_metadata
        self.report["base_patch"] = str(self.args.base.resolve())
        self.report["source_worktree_status"] = subprocess.check_output(
            ["git", "status", "--porcelain=v1"], cwd=ROOT, text=True)

    def _save(self):
        super()._save()
        AblationReport(self.out.parent).write(self.report)


class AblationCommand:
    def __init__(self, args):
        self.args = args

    def run(self):
        args = self.args
        if args.report_only:
            output = args.report_only.resolve()
            run = json.loads((output / "measurements/results.json").read_text())
            print(AblationReport(output).write(run))
            return
        if args.agent_bin is None:
            raise ValueError("--agent-bin must explicitly select the benchmark agent executable")
        if args.case:
            metadata = []
            for case in args.case:
                name, separator, path = case.partition("=")
                if not separator:
                    raise ValueError("--case requires NAME=PATCH.toml")
                metadata.append({"name": name, "patch": str(Path(path).resolve()), "parent": "0"})
        else:
            suite = args.suite.resolve()
            document = tomllib.loads(suite.read_text())
            metadata = document["cases"]
            if args.base is None and document.get("base"):
                args.base = suite.parent / document["base"]
            for case in metadata:
                if "patch" in case:
                    case["patch"] = str((suite.parent / case["patch"]).resolve())
        if args.only:
            unknown = set(args.only) - {case["name"] for case in metadata}
            if unknown:
                raise ValueError(f"unknown stages: {sorted(unknown)}")
            metadata = [case for case in metadata if case["name"] in args.only]
        sources = {}
        names = set()
        for case in metadata:
            name = case["name"]
            if (not re.fullmatch(r"[A-Za-z][A-Za-z0-9_-]*", name)
                    or name in names or name == "benchmark"):
                raise ValueError("each --case must have a unique NAME=PATCH.toml; benchmark is reserved")
            names.add(name)
            if case.get("blocked"):
                continue
            sources[name] = case["patch"]
            ConfigPatch(Path(case["patch"]))
        if args.base is None:
            args.base = HERE.parent / "configs/P.toml"
        ConfigPatch(args.base)
        output = args.out.resolve()
        output.mkdir(parents=True, exist_ok=False)
        configs = output / "inputs"
        configs.mkdir()
        (configs / "stages.json").write_text(json.dumps(metadata, indent=2, ensure_ascii=False) + "\n")
        shutil.copy2(HERE.parent / "storage_cpu/configs/benchmark.toml", configs / "benchmark.toml")
        shutil.copy2(args.base, configs / "base.snapshot.toml")
        args.storage_backends = {}
        for name, source in sources.items():
            shutil.copy2(source, configs / f"{name}.source.toml")
            patch = configs / f"{name}.toml"
            shutil.copy2(source, patch)
            ConfigPatch(args.base).apply_isolation(patch)
            backend = ConfigPatch(patch)
            selected = backend.values.get("storage", {}).get("backend", "noop")
            if selected not in ("noop", "sqlite"):
                raise ValueError(f"unsupported storage backend for {name}: {selected}")
            args.storage_backends[name] = selected
            patch.write_text(f'[storage]\nbackend = "{selected}"\n')
            backend.apply_isolation(patch)
        args.config_dir, args.out = configs, output / "measurements"
        args.modes, args.workloads = ["0", *sources], ["agent"]
        args.case_sources = sources
        args.case_metadata = metadata
        args.skip_build, args.keep_runtime, args.perf, args.backend = True, True, False, "noop"
        args.agent_bin = args.agent_bin.resolve()
        benchmark = AblationBenchmark(args)
        benchmark.report["case_metadata"] = metadata
        with (output / "run.log").open("w") as log:
            with contextlib.redirect_stdout(ProgressLog(sys.stdout, log)), \
                    contextlib.redirect_stderr(ProgressLog(sys.stderr, log)):
                try:
                    with BenchmarkLock(args.lock_path, benchmark.settings["lock_timeout_seconds"]):
                        benchmark.run()
                except BaseException as error:
                    benchmark.report.update(status="failed", error=f"{type(error).__name__}: {error}")
                    AblationReport(output).write(benchmark.report)
                    traceback.print_exc()
                    raise
                finally:
                    print(AblationReport(output).write(benchmark.report), flush=True)
                    print(f"Report: {output / 'report.md'}", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=Path("target/release"))
    parser.add_argument("--out", type=Path,
                        default=Path("local/bench/ablation") / datetime.now().strftime("%Y%m%d-%H%M%S-%f"))
    parser.add_argument("--base", type=Path, help="base patch; defaults to suite base or configs/P.toml")
    parser.add_argument("--case", action="append", help="NAME=PATCH.toml; each patch overrides --base")
    parser.add_argument("--suite", type=Path, default=HERE / "suite.toml")
    parser.add_argument("--only", nargs="+", help="run selected suite stages plus bare; report deltas only when parent is selected")
    parser.add_argument("--report-only", type=Path, help="print/regenerate an existing run's report without running agents")
    parser.add_argument("--agent-kind", choices=("xiaoo", "opencode"), default="xiaoo")
    parser.add_argument("--agent-bin", type=Path)
    parser.add_argument("--rounds", type=int)
    parser.add_argument("--warmups", type=int)
    parser.add_argument("--agent-turns", type=int)
    parser.add_argument("--agent-input-bytes", type=int)
    parser.add_argument("--agent-tpot-ms", type=float, nargs="+")
    parser.add_argument("--timeout-seconds", type=float)
    parser.add_argument("--drain-timeout-seconds", type=float)
    parser.add_argument("--cc", default="cc")
    parser.add_argument("--lock-path", type=Path, default=Path("/run/lock/actrail-v2-regression.lock"))
    AblationCommand(parser.parse_args()).run()
