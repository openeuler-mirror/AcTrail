"""Prepare Qoder CLI for the LLM capture probe."""
from __future__ import annotations

from pathlib import Path

from tests.e2e.common.agents import AgentFactory
from tests.e2e.common.core import AgentBinaryNotFoundError
from tests.e2e.common.testing_env import AgentBinaryDiscovery
from .config import ProbeQoderCliLLMConfig


class ProbeQoderCliLLMTask:
    def __init__(self, config: ProbeQoderCliLLMConfig):
        self._discovery = AgentBinaryDiscovery(config.repo)
        self._binary = self._discovery.resolve("QODERCLI_E2E_BINARY", "qodercli")
        if self._binary is None:
            raise AgentBinaryNotFoundError("Qoder CLI executable not found")

    @property
    def binary(self) -> Path:
        return self._binary

    def environment(self) -> dict[str, str]:
        return self._discovery.environment(self._binary)

    def create_agent(self):
        return AgentFactory.qodercli(
            binary=self._binary, environment=self.environment(), arguments=("--tools", ""),
        )
