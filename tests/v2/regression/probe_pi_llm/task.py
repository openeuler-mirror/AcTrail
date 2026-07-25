from __future__ import annotations

import os
import secrets
import shutil
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime, CommandResult
from tests.v2.common.errors import AgentBinaryNotFoundError

from .config import ProbePiLLMConfig


class ProbePiLLMTask:
    def __init__(self, config: ProbePiLLMConfig, runtime: ActrailRuntime):
        self._config = config
        self._runtime = runtime
        self.marker = f"A{secrets.token_hex(5)}"
        self._pi = self._resolve_pi()

    def run(self) -> CommandResult:
        return self._runtime.run(
            self._command(),
            timeout_seconds=self._config.launch_timeout_seconds,
            environment=self.environment(),
        )

    @property
    def binary(self) -> Path:
        return self._pi

    @staticmethod
    def environment() -> dict[str, str]:
        return os.environ.copy()

    def _command(self) -> list[Path | str]:
        return [
            self._runtime.actrailctl,
            "launch",
            "--",
            self._pi,
            "-p",
            f'Reply with exactly "{self.marker}" and nothing else. Do not use tools.',
            "--no-session",
        ]

    @staticmethod
    def _resolve_pi() -> Path:
        discovered = shutil.which("pi")
        if discovered:
            return Path(discovered)
        raise AgentBinaryNotFoundError("pi executable not found in PATH")
