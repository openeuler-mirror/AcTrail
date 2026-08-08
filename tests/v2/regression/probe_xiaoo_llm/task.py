from __future__ import annotations

import secrets
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime, CommandResult
from tests.v2.common.core import AgentBinaryNotFoundError
from tests.v2.common.testing_env import AgentBinaryDiscovery

from .config import ProbeXiaooLLMConfig


class ProbeXiaooLLMTask:
    def __init__(self, config: ProbeXiaooLLMConfig, runtime: ActrailRuntime):
        self._config = config
        self._runtime = runtime
        self.marker = f"A{secrets.token_hex(5)}"
        self._discovery = AgentBinaryDiscovery(config.repo)
        self._xiaoo = self._resolve_xiaoo()

    def run(self) -> CommandResult:
        return self._runtime.run(
            self._command(),
            timeout_seconds=self._config.launch_timeout_seconds,
            environment=self.environment(),
        )

    @property
    def binary(self) -> Path:
        return self._xiaoo

    def _command(self) -> list[Path | str]:
        return [
            self._runtime.actrailctl,
            "launch",
            "--",
            self._xiaoo,
            "--cli",
            "run",
            "--no-tools",
            "--max-turns",
            "1",
            "--prompt",
            f'Reply with exactly "{self.marker}" and nothing else. Do not use tools.',
        ]

    def _resolve_xiaoo(self) -> Path:
        configured = self._config.xiaoo_binary
        if configured is not None:
            if AgentBinaryDiscovery.is_executable(configured):
                return configured
            raise AgentBinaryNotFoundError(
                f"configured xiaoO executable is unavailable: "
                f"{configured}"
            )
        binary = self._discovery.resolve("XIAOO_E2E_BINARY", "xiaoo")
        if binary is None:
            raise AgentBinaryNotFoundError(
                "xiaoO executable not found; set XIAOO_E2E_BINARY to its path"
            )
        return binary

    def environment(self) -> dict[str, str]:
        return self._discovery.environment(self._xiaoo)
