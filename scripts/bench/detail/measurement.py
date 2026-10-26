"""Low-overhead wall-clock and cgroup memory measurement."""

from __future__ import annotations

import os
import signal
import subprocess
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from threading import Event, Thread
from typing import Sequence


@dataclass(frozen=True, slots=True)
class OperationSample:
    wall_seconds: float
    command_memory_peak_bytes: int
    daemon_rss_peak_bytes: int


class DaemonRssSampler:
    def __init__(self, pid: int | None, interval_seconds: float) -> None:
        self._pid = pid
        self._interval = interval_seconds
        self._stop = Event()
        self._thread = Thread(target=self._sample_until_stopped, daemon=True)
        self.peak_bytes = self._read_rss_bytes(pid)

    def start(self) -> None:
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        self._thread.join(timeout=2)
        self.peak_bytes = max(
            self.peak_bytes,
            self._read_rss_bytes(self._pid),
        )

    def _sample_until_stopped(self) -> None:
        while not self._stop.wait(self._interval):
            self.peak_bytes = max(
                self.peak_bytes,
                self._read_rss_bytes(self._pid),
            )

    @staticmethod
    def _read_rss_bytes(pid: int | None) -> int:
        if pid is None:
            return 0
        try:
            lines = Path(f"/proc/{pid}/status").read_text().splitlines()
        except OSError:
            return 0
        for line in lines:
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) * 1024
        return 0


class CgroupMeasurement:
    def __init__(
        self,
        cgroup_root: Path | None,
        helper: Path,
        *,
        sample_interval_seconds: float,
        timeout_seconds: float,
    ) -> None:
        self._root = cgroup_root
        self._helper = helper
        self._sample_interval = sample_interval_seconds
        self._timeout = timeout_seconds
        self._sequence = 0
        self._membership_name = ""
        self._peak_name = ""
        self.backend = ""

    def verify(self) -> None:
        if os.geteuid() != 0:
            raise RuntimeError("detail benchmark requires root for eBPF and cgroups")
        self._select_backend()
        if self._root is None:
            raise RuntimeError("cgroup memory controller is unavailable")
        probe = self._root / f"actrail-bench-detail-{os.getpid()}"
        try:
            probe.mkdir()
            if not (probe / self._peak_name).is_file():
                raise RuntimeError(
                    f"{self.backend} peak memory counter is unavailable"
                )
        except OSError as error:
            raise RuntimeError(
                f"cannot create benchmark cgroup below {self._root}: {error}"
            ) from error
        finally:
            try:
                probe.rmdir()
            except OSError:
                pass

    def run(
        self,
        target_command: Sequence[str],
        *,
        cwd: Path,
        elapsed_path: Path,
        launcher: Sequence[str] = (),
        daemon_pid: int | None = None,
    ) -> OperationSample:
        self._sequence += 1
        if self._root is None:
            raise RuntimeError("cgroup measurement was not verified")
        scope = self._root / (
            f"actrail-bench-detail-{os.getpid()}-{self._sequence}"
        )
        scope.mkdir()
        stderr_file = tempfile.TemporaryFile(mode="w+t", encoding="utf-8")
        if elapsed_path.exists():
            raise RuntimeError(f"workload result already exists: {elapsed_path}")
        command = [
            *launcher,
            "/bin/sh",
            str(self._helper),
            str(scope / self._membership_name),
            *target_command,
        ]
        process = subprocess.Popen(
            command,
            cwd=str(cwd),
            stdout=subprocess.DEVNULL,
            stderr=stderr_file,
            text=True,
            start_new_session=True,
        )
        daemon_sampler = DaemonRssSampler(
            daemon_pid,
            self._sample_interval,
        )
        daemon_sampler.start()
        try:
            try:
                process.wait(timeout=self._timeout)
            except subprocess.TimeoutExpired as error:
                self._terminate(process)
                raise RuntimeError(
                    f"command timed out after {self._timeout:.1f}s: "
                    + " ".join(target_command)
                ) from error
            if process.returncode != 0:
                stderr_file.seek(0)
                detail = stderr_file.read()[-3000:].strip()
                raise RuntimeError(
                    f"command exited with {process.returncode}: "
                    + " ".join(target_command)
                    + (f"\n{detail}" if detail else "")
                )
            peak = self._read_memory_peak(scope)
            elapsed = self._read_elapsed_seconds(elapsed_path)
            daemon_sampler.stop()
            return OperationSample(
                elapsed,
                peak,
                daemon_sampler.peak_bytes,
            )
        finally:
            daemon_sampler.stop()
            if process.poll() is None:
                self._terminate(process)
            stderr_file.close()
            self._cleanup_scope(scope)

    @staticmethod
    def _read_elapsed_seconds(path: Path) -> float:
        try:
            elapsed_ns = int(path.read_text(encoding="utf-8").strip())
        except (OSError, ValueError) as error:
            raise RuntimeError(
                f"cannot read workload elapsed time from {path}: {error}"
            ) from error
        if elapsed_ns <= 0:
            raise RuntimeError(f"workload elapsed time must be positive: {elapsed_ns}")
        return elapsed_ns / 1_000_000_000

    def _read_memory_peak(self, scope: Path) -> int:
        try:
            return int((scope / self._peak_name).read_text().strip())
        except (OSError, ValueError) as error:
            raise RuntimeError(
                f"cannot read cgroup memory peak for {scope}: {error}"
            ) from error

    def _select_backend(self) -> None:
        requested = self._root
        if requested is not None:
            if (requested / "cgroup.controllers").is_file():
                self._configure_v2(requested)
                return
            if requested.name == "memory" or (
                requested / "memory.usage_in_bytes"
            ).is_file():
                self._configure_v1(requested)
                return
            raise RuntimeError(
                f"cannot identify memory cgroup version at {requested}"
            )
        v2_root = Path("/sys/fs/cgroup")
        if (v2_root / "cgroup.controllers").is_file():
            self._configure_v2(v2_root)
            return
        v1_root = Path("/sys/fs/cgroup/memory")
        if (v1_root / "memory.usage_in_bytes").is_file():
            self._configure_v1(v1_root)
            return
        raise RuntimeError("no supported cgroup memory controller found")

    def _configure_v2(self, root: Path) -> None:
        self._root = root
        self._membership_name = "cgroup.procs"
        self._peak_name = "memory.peak"
        self.backend = "cgroup_v2_memory_peak"

    def _configure_v1(self, root: Path) -> None:
        self._root = root
        self._membership_name = "tasks"
        self._peak_name = "memory.max_usage_in_bytes"
        self.backend = "cgroup_v1_memory_max_usage"

    @staticmethod
    def _terminate(process: subprocess.Popen[str]) -> None:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            return
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=5)

    @staticmethod
    def _cleanup_scope(scope: Path) -> None:
        try:
            membership_names = ("cgroup.procs", "tasks")
            membership = next(
                (
                    scope / name
                    for name in membership_names
                    if (scope / name).exists()
                ),
                scope / "cgroup.procs",
            )
            remaining = membership.read_text().split()
        except OSError:
            remaining = []
        for raw_pid in remaining:
            try:
                os.kill(int(raw_pid), signal.SIGKILL)
            except (ProcessLookupError, ValueError):
                pass
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            try:
                scope.rmdir()
                return
            except OSError:
                time.sleep(0.02)
        raise RuntimeError(f"benchmark cgroup did not become empty: {scope}")
