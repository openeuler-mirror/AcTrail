from __future__ import annotations

import subprocess
from dataclasses import dataclass
from pathlib import Path

from .output import TestOutput


@dataclass(frozen=True)
class CommandResult:
    argv: tuple[str, ...]
    returncode: int
    stdout: str
    stderr: str

    @property
    def output(self) -> str:
        return self.stdout + self.stderr


class ActrailRuntime:
    def __init__(
        self,
        repo: Path,
        bin_dir: Path,
        command_timeout_seconds: int,
        output: TestOutput,
    ):
        self._repo = repo
        self._bin_dir = bin_dir if bin_dir.is_absolute() else repo / bin_dir
        self._command_timeout_seconds = command_timeout_seconds
        self._output = output
        self.actraild = self._require_binary("actraild")
        self.actrailctl = self._require_binary("actrailctl")
        self.actrailviewer = self._require_binary("actrailviewer")
        self._started = False

    def prepare(self) -> list[CommandResult]:
        results = [
            self.run_checked([self.actraild, "init", "-f"]),
            self.run_checked([self.actraild, "stop"]),
            self.run_checked([self.actrailctl, "clean"]),
            self.run_checked([self.actraild, "start"]),
        ]
        self._started = True
        return results

    def stop(self) -> CommandResult | None:
        if not self._started:
            return None
        result = self.run([self.actraild, "stop"])
        if result.returncode == 0:
            self._started = False
        return result

    def run(
        self,
        argv: list[Path | str],
        *,
        timeout_seconds: int | None = None,
        environment: dict[str, str] | None = None,
        echo: bool = True,
    ) -> CommandResult:
        command = tuple(str(argument) for argument in argv)
        completed = subprocess.run(
            command,
            cwd=self._repo,
            env=environment,
            capture_output=True,
            text=True,
            timeout=timeout_seconds or self._command_timeout_seconds,
            check=False,
        )
        if echo:
            self._output.command_output(completed.stdout, completed.stderr)
        return CommandResult(
            argv=command,
            returncode=completed.returncode,
            stdout=completed.stdout,
            stderr=completed.stderr,
        )

    def run_checked(
        self,
        argv: list[Path | str],
        *,
        timeout_seconds: int | None = None,
        environment: dict[str, str] | None = None,
        echo: bool = True,
    ) -> CommandResult:
        result = self.run(
            argv,
            timeout_seconds=timeout_seconds,
            environment=environment,
            echo=echo,
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"command exited with {result.returncode}: {' '.join(result.argv)}\n"
                f"stdout={result.stdout}\nstderr={result.stderr}"
            )
        return result

    def _require_binary(self, name: str) -> Path:
        binary = self._bin_dir / name
        if not binary.is_file():
            raise RuntimeError(f"release binary not found: {binary}")
        return binary
