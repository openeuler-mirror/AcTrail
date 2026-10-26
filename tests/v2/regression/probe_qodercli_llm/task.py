from __future__ import annotations

import os
import shutil
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime, CommandResult
from tests.v2.common.core import AgentBinaryNotFoundError

from .config import ProbeQoderCliLLMConfig


class ProbeQoderCliLLMTask:
    def __init__(self, config: ProbeQoderCliLLMConfig, runtime: ActrailRuntime):
        self._config = config
        self._runtime = runtime
        self._qodercli = self._resolve_qodercli()

    def run(self) -> CommandResult:
        return self._runtime.run(
            self._command(),
            timeout_seconds=self._config.launch_timeout_seconds,
            environment=self.environment(),
        )

    @property
    def binary(self) -> Path:
        return self._qodercli

    @staticmethod
    def environment() -> dict[str, str]:
        return os.environ.copy()

    def _command(self) -> list[Path | str]:
        return [
            self._runtime.actrailctl,
            "launch",
            "--",
            self._qodercli,
            "--no-session-persistence",
            "-p",
            "请简短回复，确认已经收到这条测试消息。",
            "--tools",
            "",
        ]

    @staticmethod
    def _resolve_qodercli() -> Path:
        discovered = shutil.which("qodercli")
        if discovered:
            return Path(discovered)
        raise AgentBinaryNotFoundError("qodercli executable not found in PATH")
