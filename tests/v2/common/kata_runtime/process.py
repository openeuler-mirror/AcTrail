from __future__ import annotations

import os
import signal
import subprocess
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol


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

    @property
    def pid(self) -> int:
        return self._process.pid

    def poll(self) -> int | None:
        return self._process.poll()

    def wait(
        self,
        *,
        timeout: float | None = None,
        terminate_grace_seconds: float = 2,
    ) -> CommandResult:
        if terminate_grace_seconds < 0:
            raise ValueError("terminate grace must be non-negative")
        try:
            stdout, stderr = self._process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            result = self.terminate(grace_seconds=terminate_grace_seconds)
            raise CommandTimeoutError(result)
        return CommandResult(
            self.argv,
            self._process.returncode,
            stdout,
            stderr,
        )

    def terminate(self, *, grace_seconds: float = 2) -> CommandResult:
        if grace_seconds < 0:
            raise ValueError("termination grace must be non-negative")
        if self._process.poll() is None:
            self._signal_group(signal.SIGTERM)
            try:
                stdout, stderr = self._process.communicate(timeout=grace_seconds)
            except subprocess.TimeoutExpired:
                self._signal_group(signal.SIGKILL)
                stdout, stderr = self._process.communicate()
        else:
            stdout, stderr = self._process.communicate()
        return CommandResult(
            self.argv,
            self._process.returncode,
            stdout,
            stderr,
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
