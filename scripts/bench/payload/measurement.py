"""Wait-based CPU accounting includes short-lived, reaped descendants."""

from __future__ import annotations

import os
import signal
import subprocess
import threading
import time
from pathlib import Path


class CommandMeasurement:
    def __init__(self, timeout_seconds: float):
        self.timeout = timeout_seconds

    def run(self, command: list[str], cwd: Path, env: dict[str, str]) -> dict:
        started = time.monotonic()
        with (cwd / "stdout.log").open("wb") as stdout, (cwd / "stderr.log").open("wb") as stderr:
            process = subprocess.Popen(
                command, cwd=cwd, env=env, stdout=stdout, stderr=stderr,
                start_new_session=True,
            )
            expired = threading.Event()

            def expire() -> None:
                expired.set()
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

            timer = threading.Timer(self.timeout, expire)
            timer.start()
            try:
                _, status, usage = os.wait4(process.pid, 0)
                finished = time.monotonic()
                process.returncode = os.waitstatus_to_exitcode(status)
            finally:
                timer.cancel()
                timer.join()
                if process.returncode is None:
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    _, status, _ = os.wait4(process.pid, 0)
                    process.returncode = os.waitstatus_to_exitcode(status)
        if expired.is_set():
            raise TimeoutError(f"workload exceeded {self.timeout:g}s")
        elapsed = (finished - started) * 1000
        if process.returncode:
            detail = (cwd / "stderr.log").read_text(errors="replace")[-1500:]
            raise RuntimeError(f"workload exited {process.returncode}: {detail}")
        return {
            "wall_ms": elapsed,
            "task_cpu_ms": (usage.ru_utime + usage.ru_stime) * 1000,
            "task_user_cpu_ms": usage.ru_utime * 1000,
            "task_system_cpu_ms": usage.ru_stime * 1000,
        }


class DaemonCpu:
    def __init__(self, pid: int):
        self.pid = pid
        self.ticks_per_second = os.sysconf("SC_CLK_TCK")
        self.start_time = self._stat()[19]

    def _stat(self) -> list[str]:
        return Path(f"/proc/{self.pid}/stat").read_text().rsplit(")", 1)[1].split()

    def read_ms(self) -> float:
        fields = self._stat()
        if fields[19] != self.start_time or fields[0] == "Z":
            raise RuntimeError("benchmark daemon exited or its PID was reused")
        return (int(fields[11]) + int(fields[12])) * 1000 / self.ticks_per_second


class DaemonProfile:
    """Optional diagnostic sampling, outside the comparable measurement series."""

    def __init__(self, pid: int | None, path: Path):
        self.pid, self.path = pid, path
        self.process = None
        self.log = None

    def artifact(self, suffix: str) -> Path:
        return self.path.parent / (self.path.name + suffix)

    def __enter__(self):
        if self.pid is not None:
            self.log = self.artifact(".log").open("wb")
            try:
                self.process = subprocess.Popen([
                    "perf", "record", "-e", "cpu-clock:u", "-F", "99",
                    "--call-graph", "dwarf,8192", "-p", str(self.pid),
                    "-o", str(self.artifact(".data")),
                ], stdout=self.log, stderr=subprocess.STDOUT)
                time.sleep(0.1)
                if self.process.poll() is not None:
                    raise RuntimeError(f"perf failed to start: {self.path}")
            except BaseException:
                if self.process is not None and self.process.poll() is None:
                    self.process.kill()
                    self.process.wait()
                self.log.close()
                raise
        return self

    def __exit__(self, exc_type, exc_value, traceback):
        if self.process is None:
            return
        self.process.send_signal(signal.SIGINT)
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
            raise RuntimeError(f"perf did not stop: {self.path}")
        finally:
            self.log.close()
        if self.process.returncode not in (0, -signal.SIGINT, 128 + signal.SIGINT):
            if exc_type is None:
                raise RuntimeError(f"perf recording failed ({self.process.returncode}): {self.path}")
            return
        with self.artifact(".txt").open("wb") as report:
            subprocess.run([
                "perf", "report", "--stdio", "--no-children", "--percent-limit", "0.5",
                "-i", str(self.artifact(".data")),
            ], stdout=report, stderr=subprocess.STDOUT, check=True, timeout=30)
