from __future__ import annotations

import atexit
import fcntl
import json
import math
import os
import socket
import stat
import subprocess
import sys
import threading
import time
from contextlib import contextmanager
from pathlib import Path
from typing import Iterator, Mapping

from tests.v2.common.testing_env import AgentAvailability

from ..core import CaseProgressReporter, TestOutput, TestResult, TestStatus


class _RegressionLockWait:
    def __init__(self, timeout_seconds: float, poll_seconds: float):
        if not math.isfinite(timeout_seconds) or timeout_seconds <= 0:
            raise ValueError("regression lock timeout must be positive")
        if not math.isfinite(poll_seconds) or poll_seconds <= 0:
            raise ValueError("regression lock poll interval must be positive")

        self.timeout_seconds = timeout_seconds
        self.poll_seconds = poll_seconds
        self._started_at = time.monotonic()
        self._deadline = self._started_at + timeout_seconds

    def remaining_seconds(self) -> float:
        return max(0.0, self._deadline - time.monotonic())

    def next_poll_seconds(self) -> float:
        return min(self.poll_seconds, self.remaining_seconds())

    def elapsed_seconds(self) -> float:
        return time.monotonic() - self._started_at


class _RegressionSuiteLock:
    def __init__(
        self,
        path: Path,
        wait: _RegressionLockWait,
        output: TestOutput,
    ):
        if not path.is_absolute():
            raise ValueError(f"regression lock path must be absolute: {path}")

        path.parent.mkdir(parents=True, exist_ok=True)
        flags = os.O_CLOEXEC | os.O_CREAT | os.O_NOFOLLOW | os.O_RDWR
        self._path = path
        descriptor = os.open(path, flags, 0o664)
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            os.close(descriptor)
            raise ValueError(f"regression lock must be a regular file: {path}")
        self._file = os.fdopen(descriptor, "r+", encoding="utf-8")
        self._output = output
        self._acquire(wait)
        atexit.register(self.close)

    def close(self) -> None:
        if self._file.closed:
            return
        try:
            fcntl.flock(self._file.fileno(), fcntl.LOCK_UN)
        finally:
            self._file.close()

    def _acquire(self, wait: _RegressionLockWait) -> None:
        waiting_reported = False
        while True:
            remaining = wait.remaining_seconds()
            if remaining <= 0:
                owner = self._owner_description()
                self._file.close()
                raise TimeoutError(
                    "timed out waiting for regression lock; "
                    "regression test did not start "
                    f"path={self._path} holder={owner} "
                    f"timeout_seconds={wait.timeout_seconds:g}"
                )
            try:
                fcntl.flock(
                    self._file.fileno(),
                    fcntl.LOCK_EX | fcntl.LOCK_NB,
                )
                break
            except BlockingIOError:
                if not waiting_reported:
                    self._output.line(
                        "waiting for regression lock "
                        f"path={self._path} holder={self._owner_description()} "
                        f"timeout_seconds={wait.timeout_seconds:g}"
                    )
                    waiting_reported = True
                time.sleep(wait.next_poll_seconds())

        self._write_owner()
        self._output.line(
            "acquired regression lock "
            f"path={self._path} pid={os.getpid()} "
            f"wait_seconds={wait.elapsed_seconds():.3f}"
        )

    def _write_owner(self) -> None:
        owner = {
            "argv": sys.argv,
            "hostname": socket.gethostname(),
            "pid": os.getpid(),
            "started_unix_seconds": time.time(),
        }
        self._file.seek(0)
        self._file.truncate()
        json.dump(owner, self._file, sort_keys=True)
        self._file.write("\n")
        self._file.flush()

    def _owner_description(self) -> str:
        try:
            self._file.seek(0)
            owner = json.loads(self._file.read())
        except (OSError, TypeError, ValueError):
            return "unknown"
        pid = owner.get("pid")
        if not isinstance(pid, int):
            return "unknown"
        command = owner.get("argv")
        command_text = (
            " ".join(str(part) for part in command)
            if isinstance(command, list)
            else "unknown"
        )
        return (
            f"pid={pid},alive={str(self._pid_is_alive(pid)).lower()},"
            f"command={command_text}"
        )

    @staticmethod
    def _pid_is_alive(pid: int) -> bool:
        try:
            os.kill(pid, 0)
            return True
        except ProcessLookupError:
            return False
        except PermissionError:
            return True


class TestingContextSingleton:
    _instance = None
    _lease_guard = threading.RLock()
    _lease_owner_thread_id: int | None = None
    _lease_owner_thread_name: str | None = None

    def __new__(
        cls,
        *,
        lock_path: Path,
        lock_timeout_seconds: float,
        lock_poll_seconds: float,
        output: TestOutput,
    ):
        wait = _RegressionLockWait(lock_timeout_seconds, lock_poll_seconds)
        guard_acquired = cls._lease_guard.acquire(blocking=False)
        if not guard_acquired:
            output.line(
                "waiting for in-process regression lease "
                f"path={lock_path} holder={cls._lease_owner_description()} "
                f"timeout_seconds={wait.timeout_seconds:g}"
            )
            guard_acquired = cls._lease_guard.acquire(
                timeout=wait.remaining_seconds()
            )
        if not guard_acquired:
            holder = cls._lease_owner_description()
            raise TimeoutError(
                "timed out waiting for in-process regression lease; "
                "regression test did not start "
                f"path={lock_path} holder={holder} "
                f"timeout_seconds={wait.timeout_seconds:g}"
            )
        try:
            if cls._instance is None:
                current_thread = threading.current_thread()
                cls._lease_owner_thread_id = current_thread.ident
                cls._lease_owner_thread_name = current_thread.name
                instance = super(TestingContextSingleton, cls).__new__(cls)
                instance._env_dict = {}
                instance.agent_availability = AgentAvailability()
                instance._output_stack = [output]
                instance._progress_stack = []
                instance._lock_path = lock_path
                instance._lease_depth = 1
                instance._release_prepared = False
                instance._release_repo = None
                instance._suite_lock = _RegressionSuiteLock(
                    lock_path,
                    wait,
                    output,
                )
                cls._instance = instance
            else:
                if cls._instance._lock_path != lock_path:
                    raise ValueError(
                        "nested regression run requested a different lock path: "
                        f"held={cls._instance._lock_path} requested={lock_path}"
                    )
                cls._instance._lease_depth += 1
            return cls._instance
        except BaseException:
            if cls._instance is None:
                cls._lease_owner_thread_id = None
                cls._lease_owner_thread_name = None
            cls._lease_guard.release()
            raise

    def close(self) -> None:
        cls = type(self)
        try:
            self._lease_depth -= 1
            if self._lease_depth == 0:
                self._suite_lock.close()
                cls._instance = None
                cls._lease_owner_thread_id = None
                cls._lease_owner_thread_name = None
        finally:
            cls._lease_guard.release()

    @property
    def output(self) -> TestOutput:
        return self._output_stack[-1]

    def prepare_release(self, repo: Path) -> None:
        resolved_repo = repo.resolve()
        if (
            self._release_repo is not None
            and self._release_repo != resolved_repo
        ):
            raise RuntimeError(
                "regression singleton cannot prepare releases from "
                f"multiple repositories: held={self._release_repo} "
                f"requested={resolved_repo}"
            )
        if self._release_prepared:
            return

        self._release_repo = resolved_repo
        script = resolved_repo / "scripts/install-release.sh"
        if not script.is_file():
            raise RuntimeError(f"release installer not found: {script}")

        self.output.heading("▶ release_install")
        self.output.line("→ running bash scripts/install-release.sh")
        try:
            completed = subprocess.run(
                ["bash", "scripts/install-release.sh"],
                cwd=resolved_repo,
                capture_output=True,
                text=True,
                check=False,
            )
        except OSError as error:
            raise RuntimeError(
                f"launch release installer failed: {error}"
            ) from error
        if completed.returncode != 0:
            self.output.command_output(
                completed.stdout,
                completed.stderr,
            )
            raise RuntimeError(
                "bash scripts/install-release.sh exited with "
                f"{completed.returncode}"
            )
        self._release_prepared = True
        self.output.summary(
            "release_install",
            TestResult(TestStatus.PASSED),
        )

    @contextmanager
    def output_scope(self, output: TestOutput) -> Iterator[None]:
        self._output_stack.append(output)
        try:
            yield
        finally:
            active_output = self._output_stack.pop()
            if active_output is not output:
                raise RuntimeError("regression output scope stack is corrupted")

    @contextmanager
    def progress_scope(
        self,
        reporter: CaseProgressReporter,
    ) -> Iterator[None]:
        self._progress_stack.append(reporter)
        try:
            yield
        finally:
            active_reporter = self._progress_stack.pop()
            if active_reporter is not reporter:
                raise RuntimeError(
                    "regression progress scope stack is corrupted"
                )

    def report_progress(
        self,
        step: str,
        message: str | None = None,
    ) -> None:
        if not self._progress_stack:
            raise RuntimeError("regression progress scope is inactive")
        self._progress_stack[-1].report(step, message)

    @classmethod
    def _lease_owner_description(cls) -> str:
        if cls._lease_owner_thread_id is None:
            return "unknown"
        return (
            f"thread_id={cls._lease_owner_thread_id},"
            f"thread_name={cls._lease_owner_thread_name or 'unknown'}"
        )

    def check_agent_availability(
        self,
        agent_name: str,
        binary: Path | str | None = None,
        environment: Mapping[str, str] | None = None,
    ) -> bool:
        return self.agent_availability.check_agent_availability(
            agent_name,
            binary,
            environment,
        )
