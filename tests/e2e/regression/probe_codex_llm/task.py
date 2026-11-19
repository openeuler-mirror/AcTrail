"""Prepare Codex discovery and scenario arguments for the LLM probe."""
from __future__ import annotations

import json
from pathlib import Path

from tests.e2e.common.agents import AgentFactory
from tests.e2e.common.core import AgentBinaryNotFoundError
from tests.e2e.common.testing_env import AgentBinaryDiscovery

from .config import ProbeCodexLLMConfig


class ProbeCodexLLMTask:
    def __init__(self, config: ProbeCodexLLMConfig):
        self._config = config
        self._reply_file = config.work_dir / "codex-reply.txt"
        self._discovery = AgentBinaryDiscovery(config.repo)
        self._codex = self._resolve_codex()
        self._native_codex = self._resolve_native_codex()

    def create_agent(self):
        """场景参数透传给 Codex，启动与协议由适配器封装。"""
        return AgentFactory.codex(
            binary=self._codex,
            arguments=(
                "--skip-git-repo-check",
                "-c", f"model={json.dumps(self._config.model)}",
                "-c", f"model_reasoning_effort={self._config.reasoning_effort}",
                "--output-last-message", str(self._reply_file),
            ),
            environment=self.environment(),
        )

    def read_reply(self) -> str:
        return self._reply_file.read_text(encoding="utf-8")

    @property
    def binary(self) -> Path:
        return self._codex

    @property
    def native_binary(self) -> Path:
        return self._native_codex

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
        return self._discovery.codex_environment(self._codex)
