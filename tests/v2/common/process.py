from __future__ import annotations

import os
import signal
import subprocess
import threading
import time
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol, TextIO


@dataclass(frozen=True)
class CommandResult:
    argv: tuple[str, ...]
    returncode: int
    stdout: str
    stderr: str

    @property
    def diagnostic(self) -> str:
        return (self.stderr or self.stdout).strip()


class CommandRunner(Protocol):
    def run(
        self,
        argv: Sequence[str],
        *,
        timeout: float | None = None,
        cwd: Path | None = None,
        environment: Mapping[str, str] | None = None,
        input_text: str | None = None,
    ) -> CommandResult: ...


class ProcessRunner(CommandRunner, Protocol):
    def start(
        self,
        argv: Sequence[str],
        *,
        cwd: Path | None = None,
        environment: Mapping[str, str] | None = None,
    ) -> ManagedProcess: ...


class CommandTimeoutError(RuntimeError):
    def __init__(self, result: CommandResult):
        self.result = result
        super().__init__(
            "command timed out and its process group was terminated: "
            + " ".join(result.argv)
        )


class ManagedProcess:
    """Owns a subprocess and the independent process group created for it."""

    def __init__(
        self,
        argv: tuple[str, ...],
        process: subprocess.Popen[str],
    ) -> None:
        self.argv = argv
        self._process = process
        if process.stdout is None or process.stderr is None:
            raise ValueError("managed process requires captured stdout and stderr")
        self._output_condition = threading.Condition()
        self._stdout: list[str] = []
        self._stderr: list[str] = []
        self._output_threads = (
            self._start_output_thread(process.stdout, self._stdout),
            self._start_output_thread(process.stderr, self._stderr),
        )

    @property
    def pid(self) -> int:
        return self._process.pid

    def poll(self) -> int | None:
        return self._process.poll()

    def wait_for_output(self, marker: str, *, timeout: float) -> None:
        """Wait until either captured stream contains an exact text marker."""

        if not marker:
            raise ValueError("managed process output marker must not be empty")
        if timeout <= 0:
            raise ValueError("managed process output timeout must be positive")
        deadline = time.monotonic() + timeout
        while True:
            with self._output_condition:
                if self._contains_output(marker):
                    return
                returncode = self._process.poll()
                remaining = deadline - time.monotonic()
                if returncode is None and remaining > 0:
                    self._output_condition.wait(timeout=remaining)
                    continue
            if returncode is not None:
                self._join_output_threads()
                with self._output_condition:
                    if self._contains_output(marker):
                        return
                    diagnostic = self._diagnostic()
                raise RuntimeError(
                    "managed process exited before output marker "
                    f"{marker!r} exit={returncode}: "
                    f"{diagnostic or 'no diagnostic output'}"
                )
            with self._output_condition:
                diagnostic = self._diagnostic()
            raise TimeoutError(
                f"timed out waiting for managed process output {marker!r}: "
                f"{diagnostic or 'no diagnostic output'}"
            )

    def wait(
        self,
        *,
        timeout: float | None = None,
        terminate_grace_seconds: float = 2,
    ) -> CommandResult:
        if terminate_grace_seconds < 0:
            raise ValueError("terminate grace must be non-negative")
        try:
            self._process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            result = self.terminate(grace_seconds=terminate_grace_seconds)
            raise CommandTimeoutError(result)
        self._join_output_threads()
        return self._result()

    def terminate(self, *, grace_seconds: float = 2) -> CommandResult:
        if grace_seconds < 0:
            raise ValueError("termination grace must be non-negative")
        if self._process.poll() is None:
            self._signal_group(signal.SIGTERM)
            try:
                self._process.wait(timeout=grace_seconds)
            except subprocess.TimeoutExpired:
                self._signal_group(signal.SIGKILL)
                self._process.wait()
        self._join_output_threads()
        return self._result()

    def _start_output_thread(
        self,
        stream: TextIO,
        chunks: list[str],
    ) -> threading.Thread:
        thread = threading.Thread(
            target=self._drain_output,
            args=(stream, chunks),
            daemon=True,
        )
        thread.start()
        return thread

    def _drain_output(self, stream: TextIO, chunks: list[str]) -> None:
        try:
            for line in iter(stream.readline, ""):
                with self._output_condition:
                    chunks.append(line)
                    self._output_condition.notify_all()
        finally:
            stream.close()
            with self._output_condition:
                self._output_condition.notify_all()

    def _join_output_threads(self) -> None:
        for thread in self._output_threads:
            thread.join()

    def _contains_output(self, marker: str) -> bool:
        return marker in "".join(self._stdout) or marker in "".join(self._stderr)

    def _diagnostic(self) -> str:
        return ("".join(self._stderr) or "".join(self._stdout)).strip()

    def _result(self) -> CommandResult:
        returncode = self._process.returncode
        if returncode is None:
            raise RuntimeError("managed process result requested before exit")
        return CommandResult(
            self.argv,
            returncode,
            "".join(self._stdout),
            "".join(self._stderr),
        )

    def _signal_group(self, sig: int) -> None:
        try:
            os.killpg(os.getpgid(self._process.pid), sig)
        except ProcessLookupError:
            return


class SubprocessRunner:
    """Runs argv-only commands without invoking a shell."""

    def __init__(self, *, terminate_grace_seconds: float = 2) -> None:
        if terminate_grace_seconds < 0:
            raise ValueError("termination grace must be non-negative")
        self._terminate_grace_seconds = terminate_grace_seconds

    def run(
        self,
        argv: Sequence[str],
        *,
        timeout: float | None = None,
        cwd: Path | None = None,
        environment: Mapping[str, str] | None = None,
        input_text: str | None = None,
    ) -> CommandResult:
        command = tuple(str(value) for value in argv)
        process = subprocess.Popen(
            command,
            cwd=cwd,
            env=(os.environ | dict(environment)) if environment else None,
            text=True,
            stdin=subprocess.PIPE if input_text is not None else None,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
        try:
            stdout, stderr = process.communicate(input_text, timeout=timeout)
        except subprocess.TimeoutExpired:
            _signal_process_group(process, signal.SIGTERM)
            try:
                stdout, stderr = process.communicate(
                    timeout=self._terminate_grace_seconds
                )
            except subprocess.TimeoutExpired:
                _signal_process_group(process, signal.SIGKILL)
                stdout, stderr = process.communicate()
            raise CommandTimeoutError(
                CommandResult(command, process.returncode, stdout, stderr)
            )
        return CommandResult(
            command,
            process.returncode,
            stdout,
            stderr,
        )

    def start(
        self,
        argv: Sequence[str],
        *,
        cwd: Path | None = None,
        environment: Mapping[str, str] | None = None,
    ) -> ManagedProcess:
        command = tuple(str(value) for value in argv)
        process = subprocess.Popen(
            command,
            cwd=cwd,
            env=(os.environ | dict(environment)) if environment else None,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
        return ManagedProcess(command, process)


def _signal_process_group(process: subprocess.Popen[str], sig: int) -> None:
    try:
        os.killpg(os.getpgid(process.pid), sig)
    except ProcessLookupError:
        return
