"""Prepare Dcode probe inputs and scenario arguments."""
from __future__ import annotations

from pathlib import Path

from tests.e2e.common.agents import AgentFactory
from tests.e2e.common.core import AgentBinaryNotFoundError
from tests.e2e.common.testing_env import AgentBinaryDiscovery
from .config import ProbeDcodeLLMConfig


class ProbeDcodeLLMTask:
    def __init__(self, config: ProbeDcodeLLMConfig):
        self._config = config
        self._discovery = AgentBinaryDiscovery(config.repo)
        self._binary = self._discovery.resolve("DCODE_E2E_BINARY", "dcode")
        if self._binary is None:
            raise AgentBinaryNotFoundError("Dcode executable not found")

    @property
    def binary(self) -> Path:
        return self._binary

    def environment(self) -> dict[str, str]:
        return self._discovery.environment(self._binary)

    def create_agent(self):
        return AgentFactory.dcode(
            binary=self._binary, environment=self.environment(),
            arguments=("--no-mcp", "--timeout", str(self._config.launch_timeout_seconds)),
            shutdown_timeout_seconds=self._config.command_timeout_seconds,
        )

