from __future__ import annotations

import secrets
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime, CommandResult
from tests.v2.common.core import AgentBinaryNotFoundError
from tests.v2.common.testing_env import AgentBinaryDiscovery

from .config import ProbeCodexLLMConfig


class ProbeCodexLLMTask:
    def __init__(self, config: ProbeCodexLLMConfig):
        self._config = config
        self.marker = f"A{secrets.token_hex(5)}"
        self._discovery = AgentBinaryDiscovery(config.repo)
        self._codex = self._resolve_codex()
        self._native_codex = self._resolve_native_codex()

    def run(self, runtime: ActrailRuntime) -> CommandResult:
        return runtime.run(
            self._command(runtime),
            timeout_seconds=self._config.launch_timeout_seconds,
            environment=self.environment(),
        )

    @property
    def binary(self) -> Path:
        return self._codex

    @property
    def native_binary(self) -> Path:
        return self._native_codex

    def _command(self, runtime: ActrailRuntime) -> list[Path | str]:
        return [
            *runtime.control_command("launch"),
            "--",
            self._codex,
            "exec",
            "--ephemeral",
            "-m",
            self._config.model,
            "-c",
            f"model_reasoning_effort={self._config.reasoning_effort}",
            f'Reply with exactly "{self.marker}" and nothing else. Do not use tools.',
        ]

    def _resolve_codex(self) -> Path:
        configured = self._config.codex_binary
        if configured is not None:
            if AgentBinaryDiscovery.is_executable(configured):
                return configured
            raise AgentBinaryNotFoundError(
                "configured Codex executable is unavailable: "
                f"{configured}"
            )
        binary = self._discovery.resolve("CODEX_E2E_BINARY", "codex")
        if binary is None:
            raise AgentBinaryNotFoundError(
                "Codex executable not found; set CODEX_E2E_BINARY to its path"
            )
        return binary

    def _resolve_native_codex(self) -> Path:
        binary = self._discovery.codex_native_binary_for(self._codex)
        if binary is None:
            raise AgentBinaryNotFoundError(
                "native Codex executable not found; set "
                "CODEX_E2E_NATIVE_BINARY to its path"
            )
        return binary

    def environment(self) -> dict[str, str]:
        environment = self._discovery.environment(self._codex)
        for parent in self._codex.resolve().parents:
            if parent.name == ".codex":
                environment["HOME"] = str(parent.parent)
                environment["CODEX_HOME"] = str(parent)
                break
        return environment
