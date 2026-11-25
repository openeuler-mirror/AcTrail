"""Prepare XiaoO discovery and scenario arguments for the LLM probe."""
from __future__ import annotations

from pathlib import Path

from tests.e2e.common.agents import AgentFactory
from tests.e2e.common.core import AgentBinaryNotFoundError
from tests.e2e.common.testing_env import AgentBinaryDiscovery

from .config import ProbeXiaooLLMConfig

from .constants import CASE_CONSTANTS


class ProbeXiaooLLMTask:
    def __init__(self, config: ProbeXiaooLLMConfig):
        self._config = config
        self._discovery = AgentBinaryDiscovery(config.repo)
        self._xiaoo = self._resolve_xiaoo()

    def create_agent(self):
        """场景参数透传给 XiaoO，进程生命周期由适配器负责。"""
        return AgentFactory.xiaoo(
            binary=self._xiaoo,
            arguments=("--no-tools", "--max-turns", str(CASE_CONSTANTS.max_turns)),
            environment=self.environment(),
        )

    @property
    def binary(self) -> Path:
        return self._xiaoo

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
