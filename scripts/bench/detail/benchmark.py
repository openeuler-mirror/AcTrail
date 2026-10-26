"""Orchestrate operation-level bare versus AcTrail measurements."""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import shutil
import statistics
import tempfile
import time
from contextlib import AbstractContextManager
from dataclasses import asdict
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Thread
from typing import Sequence

from scripts.bench.detail.measurement import (
    CgroupMeasurement,
    OperationSample,
)
from scripts.bench.detail.workload import (
    OPERATION_NAMES,
    WorkloadCase,
    WorkloadSuite,
)
from scripts.bench.overall.runtime import (
    ReleaseBuild,
    prepare_actrail,
    stop_actrail,
)


REPO_ROOT = Path(__file__).resolve().parents[3]
DEFAULT_COUNTS = {
    "file-write": 5000,
    "file-read": 10000,
    "bash-light": 200,
    "network": 1000,
    "bash-heavy": 10,
    "mixed": 12,
}
LABELS = {
    "file-write": "file write",
    "file-read": "file read",
    "bash-light": "bash light",
    "network": "HTTP local",
    "bash-heavy": "bash compile",
    "mixed": "mixed",
}


class BenchmarkLock(AbstractContextManager["BenchmarkLock"]):
    def __init__(self, path: Path, timeout_seconds: float) -> None:
        self._path = path
        self._timeout = timeout_seconds
        self._file = None

    def __enter__(self) -> "BenchmarkLock":
        self._path.parent.mkdir(parents=True, exist_ok=True)
        self._file = self._path.open("a+")
        deadline = time.monotonic() + self._timeout
        while True:
            try:
                fcntl.flock(self._file.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                return self
            except BlockingIOError:
                if time.monotonic() >= deadline:
                    self._file.close()
                    self._file = None
                    raise RuntimeError(
                        f"timed out waiting for benchmark lock {self._path}"
                    )
                time.sleep(0.5)

    def __exit__(self, *_args: object) -> None:
        if self._file is not None:
            fcntl.flock(self._file.fileno(), fcntl.LOCK_UN)
            self._file.close()


class LocalHttpServer(AbstractContextManager["LocalHttpServer"]):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def do_GET(self) -> None:
            self.send_response(204)
            self.send_header("Content-Length", "0")
            self.end_headers()

        def log_message(self, _format: str, *_args: object) -> None:
            return

    def __init__(self) -> None:
        self._server = ThreadingHTTPServer(("127.0.0.1", 0), self.Handler)
        self._thread = Thread(target=self._server.serve_forever, daemon=True)

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self._server.server_port}/ping"

    def __enter__(self) -> "LocalHttpServer":
        self._thread.start()
        return self

    def __exit__(self, *_args: object) -> None:
        self._server.shutdown()
        self._server.server_close()
        self._thread.join(timeout=5)


class DetailBenchmark:
    def __init__(self, args: argparse.Namespace, work_dir: Path) -> None:
        self._args = args
        self._work_dir = work_dir
        self._bin_dir = args.bin_dir.resolve()
        self._measurement = CgroupMeasurement(
            args.cgroup_root,
            Path(__file__).with_name("cgroup-exec.sh"),
            sample_interval_seconds=args.sample_interval_ms / 1000,
            timeout_seconds=args.timeout_seconds,
        )
        self._cases = [
            WorkloadCase(
                name,
                LABELS[name],
                getattr(args, name.replace("-", "_") + "_count"),
            )
            for name in args.operations
        ]
        self._suite: WorkloadSuite | None = None
        self._sample_sequence = 0

    def run(self, network_url: str) -> dict[str, object]:
        self._measurement.verify()
        self._verify_inputs()
        self._suite = WorkloadSuite(
            self._work_dir / "workloads",
            file_bytes=self._args.file_bytes,
            compiler=self._args.compiler,
            network_url=network_url,
        )
        if self._args.calibrate_only:
            print(
                f"memory backend: {self._measurement.backend}",
                flush=True,
            )
            samples = self._run_phase(observed=False, daemon_pid=None)
            self._print_calibration(samples)
            return {"calibration": True}
        self._require_no_running_actraild()
        commit = self._prepare_release()
        self._verify_actrail_binaries()

        print("phase: bare (actraild stopped)", flush=True)
        bare = self._run_phase(observed=False, daemon_pid=None)

        runtime_dir = self._work_dir / "runtime"
        runtime_dir.mkdir()
        daemon_pid: int | None = None
        try:
            daemon_pid = prepare_actrail(runtime_dir, self._bin_dir)
            print("phase: observed (actrailctl launch)", flush=True)
            observed = self._run_phase(observed=True, daemon_pid=daemon_pid)
        finally:
            stop_actrail(runtime_dir, self._bin_dir)

        self._print_table(bare, observed)
        report = self._report(commit, bare, observed)
        if self._args.out is not None:
            self._args.out.parent.mkdir(parents=True, exist_ok=True)
            self._args.out.write_text(
                json.dumps(report, ensure_ascii=False, indent=2) + "\n",
                encoding="utf-8",
            )
            print(f"report: {self._args.out}")
        return report

    def _prepare_release(self) -> dict[str, str]:
        release = ReleaseBuild(REPO_ROOT)
        if self._args.skip_build:
            commit = release.commit_info()
        else:
            commit = release.ensure(
                timeout_seconds=self._args.build_timeout_seconds,
            )
        print(f"commit: {commit['id'][:8]} {commit['title']}", flush=True)
        return commit

    def _verify_inputs(self) -> None:
        if self._args.rounds < 1:
            raise RuntimeError("--rounds must be at least 1")
        if self._args.warmups < 0:
            raise RuntimeError("--warmups cannot be negative")
        if self._args.file_bytes < 1:
            raise RuntimeError("--file-bytes must be positive")
        if self._args.sample_interval_ms <= 0:
            raise RuntimeError("--sample-interval-ms must be positive")
        compiler = shutil.which(self._args.compiler)
        if compiler is None:
            raise RuntimeError(f"compiler not found: {self._args.compiler}")
        self._args.compiler = compiler
        for case in self._cases:
            if case.count < 1:
                raise RuntimeError(f"count for {case.name} must be positive")

    def _verify_actrail_binaries(self) -> None:
        for binary in ("actraild", "actrailctl"):
            path = self._bin_dir / binary
            if not path.is_file():
                raise RuntimeError(f"release binary not found: {path}")
    def _run_phase(
        self,
        *,
        observed: bool,
        daemon_pid: int | None,
    ) -> dict[str, list[OperationSample]]:
        samples: dict[str, list[OperationSample]] = {}
        for case in self._cases:
            for _ in range(self._args.warmups):
                self._run_case(case, observed=observed, daemon_pid=daemon_pid)
            measured = [
                self._run_case(case, observed=observed, daemon_pid=daemon_pid)
                for _ in range(self._args.rounds)
            ]
            samples[case.name] = measured
        return samples

    def _run_case(
        self,
        case: WorkloadCase,
        *,
        observed: bool,
        daemon_pid: int | None,
    ) -> OperationSample:
        if self._suite is None:
            raise RuntimeError("workload suite is not initialized")
        self._suite.prepare(case)
        self._sample_sequence += 1
        result_dir = self._work_dir / "elapsed"
        result_dir.mkdir(exist_ok=True)
        elapsed_path = result_dir / f"sample-{self._sample_sequence}.ns"
        target_command = self._suite.command(case, elapsed_path)
        launcher: list[str] = []
        if observed:
            launcher = [
                str(self._bin_dir / "actrailctl"),
                "--config",
                str(self._work_dir / "runtime" / "actraild.conf"),
                "launch",
                "--",
            ]
        return self._measurement.run(
            target_command,
            cwd=REPO_ROOT,
            elapsed_path=elapsed_path,
            launcher=launcher,
            daemon_pid=daemon_pid,
        )

    @staticmethod
    def _require_no_running_actraild() -> None:
        running: list[str] = []
        for entry in Path("/proc").iterdir():
            if not entry.name.isdigit():
                continue
            try:
                if (entry / "comm").read_text().strip() == "actraild":
                    running.append(entry.name)
            except OSError:
                continue
        if running:
            raise RuntimeError(
                "bare measurement requires actraild to be stopped; running pid(s): "
                + ", ".join(running)
            )

    def _print_table(
        self,
        bare: dict[str, list[OperationSample]],
        observed: dict[str, list[OperationSample]],
    ) -> None:
        header = (
            f"{'operation':<14} {'N':>7} {'bare ms':>10} "
            f"{'observed ms':>11} {'time Δ':>9} {'bare MiB':>10} "
            f"{'observed MiB':>12} {'daemon MiB':>10}"
        )
        print(header)
        print("-" * len(header))
        for case in self._cases:
            bare_wall = self._mean(bare[case.name], "wall_seconds") * 1000
            observed_wall = self._mean(
                observed[case.name], "wall_seconds"
            ) * 1000
            overhead = (observed_wall / bare_wall - 1) * 100
            bare_memory = self._mean(
                bare[case.name], "command_memory_peak_bytes"
            ) / (1024 * 1024)
            observed_memory = self._mean(
                observed[case.name], "command_memory_peak_bytes"
            ) / (1024 * 1024)
            daemon_memory = self._mean(
                observed[case.name], "daemon_rss_peak_bytes"
            ) / (1024 * 1024)
            print(
                f"{case.label:<14} {case.count:>7} {bare_wall:>10.1f} "
                f"{observed_wall:>11.1f} {overhead:>+8.1f}% "
                f"{bare_memory:>10.1f} {observed_memory:>12.1f} "
                f"{daemon_memory:>10.1f}"
            )

    def _print_calibration(
        self,
        samples: dict[str, list[OperationSample]],
    ) -> None:
        print(f"{'operation':<14} {'N':>7} {'mean ms':>10}")
        print("-" * 33)
        for case in self._cases:
            wall_ms = self._mean(samples[case.name], "wall_seconds") * 1000
            print(f"{case.label:<14} {case.count:>7} {wall_ms:>10.1f}")

    def _report(
        self,
        commit: dict[str, str],
        bare: dict[str, list[OperationSample]],
        observed: dict[str, list[OperationSample]],
    ) -> dict[str, object]:
        return {
            "commit": commit,
            "rounds": self._args.rounds,
            "warmups_discarded": self._args.warmups,
            "memory_measurement": {
                "command": self._measurement.backend,
                "daemon": "sampled_vm_rss",
                "sample_interval_ms": self._args.sample_interval_ms,
            },
            "counts": {case.name: case.count for case in self._cases},
            "samples": {
                case.name: {
                    "bare": [asdict(item) for item in bare[case.name]],
                    "observed": [
                        asdict(item) for item in observed[case.name]
                    ],
                }
                for case in self._cases
            },
        }

    @staticmethod
    def _mean(samples: list[OperationSample], field: str) -> float:
        return statistics.mean(getattr(sample, field) for sample in samples)


def create_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Compare operation costs without and with AcTrail observation",
    )
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument(
        "--operations",
        nargs="+",
        choices=OPERATION_NAMES,
        default=list(OPERATION_NAMES),
    )
    for name, count in DEFAULT_COUNTS.items():
        parser.add_argument(
            f"--{name}-count",
            type=int,
            default=count,
            help=f"operations per {name} sample (default: {count})",
        )
    parser.add_argument("--file-bytes", type=int, default=4096)
    parser.add_argument("--compiler", default="cc")
    parser.add_argument(
        "--bin-dir",
        type=Path,
        default=REPO_ROOT / "target/release",
    )
    parser.add_argument(
        "--cgroup-root",
        type=Path,
        default=None,
        help="memory cgroup mount; default auto-detects v1 or v2",
    )
    parser.add_argument("--sample-interval-ms", type=float, default=10.0)
    parser.add_argument("--timeout-seconds", type=float, default=120.0)
    parser.add_argument("--build-timeout-seconds", type=float, default=3600.0)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument(
        "--calibrate-only",
        action="store_true",
        help="measure workload duration only; do not build or touch actraild",
    )
    parser.add_argument("--out", type=Path)
    parser.add_argument(
        "--lock-path",
        type=Path,
        default=Path("/run/lock/actrail-v2-regression.lock"),
    )
    parser.add_argument("--lock-timeout-seconds", type=float, default=900.0)
    parser.add_argument("--keep-work-dir", action="store_true")
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = create_parser().parse_args(argv)
    work_dir = Path(tempfile.mkdtemp(prefix="actrail-bench-detail-"))
    failed = False
    try:
        if args.calibrate_only:
            with LocalHttpServer() as server:
                DetailBenchmark(args, work_dir).run(server.url)
        else:
            with BenchmarkLock(args.lock_path, args.lock_timeout_seconds):
                with LocalHttpServer() as server:
                    DetailBenchmark(args, work_dir).run(server.url)
        return 0
    except Exception:
        failed = True
        raise
    finally:
        if args.keep_work_dir or failed:
            print(f"work dir: {work_dir}")
        else:
            shutil.rmtree(work_dir, ignore_errors=True)
