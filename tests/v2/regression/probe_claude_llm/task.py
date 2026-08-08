from __future__ import annotations

import secrets
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime, CommandResult
from tests.v2.common.core import AgentBinaryNotFoundError
from tests.v2.common.testing_env import AgentBinaryDiscovery

from .config import ProbeClaudeLLMConfig


class ProbeClaudeLLMTask:
    def __init__(self, config: ProbeClaudeLLMConfig, runtime: ActrailRuntime):
        self._config = config
        self._runtime = runtime
        self.marker = f"A{secrets.token_hex(5)}"
        self._discovery = AgentBinaryDiscovery(config.repo)
        self._claude = self._resolve_claude()

    def run(self) -> CommandResult:
        return self._runtime.run(
            self._command(),
            timeout_seconds=self._config.launch_timeout_seconds,
            environment=self.environment(),
        )

    @property
    def binary(self) -> Path:
        return self._claude

    def _command(self) -> list[Path | str]:
        return [
            self._runtime.actrailctl,
            "launch",
            "--",
            self._claude,
            f'Reply with exactly "{self.marker}" and nothing else. Do not use tools.',
            "--print",
            "--output-format",
            "text",
            "--model",
            self._config.model,
            "--no-session-persistence",
            "--safe-mode",
            "--permission-mode",
            "dontAsk",
            "--tools",
            "",
        ]

    def _resolve_claude(self) -> Path:
        configured = self._config.claude_binary
        if configured is not None:
            if AgentBinaryDiscovery.is_executable(configured):
                return configured
            raise AgentBinaryNotFoundError(
                "configured Claude executable is unavailable: "
                f"{configured}"
            )
        binary = self._discovery.resolve("CLAUDE_E2E_BINARY", "claude")
        if binary is None:
            raise AgentBinaryNotFoundError(
                "Claude executable not found; set CLAUDE_E2E_BINARY to its path"
            )
        return binary

    def environment(self) -> dict[str, str]:
        return self._discovery.environment(self._claude)
