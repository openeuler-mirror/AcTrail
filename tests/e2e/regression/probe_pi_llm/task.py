"""Prepare Pi discovery for the LLM probe."""
from __future__ import annotations

import os
import shutil
from pathlib import Path

from tests.e2e.common.agents import AgentFactory
from tests.e2e.common.core import AgentBinaryNotFoundError

from .config import ProbePiLLMConfig


class ProbePiLLMTask:
    def __init__(self, config: ProbePiLLMConfig):
        self._config = config
        self._pi = self._resolve_pi()

    def create_agent(self):
        """Pi 协议和启动命令由具体适配器封装。"""
        return AgentFactory.pi(
            binary=self._pi,
            arguments=("--no-tools", "--no-mcp"),
            environment=self.environment(),
        )

    @property
    def binary(self) -> Path:
        return self._pi

    @staticmethod
    def environment() -> dict[str, str]:
        return os.environ.copy()

    @staticmethod
    def _resolve_pi() -> Path:
        discovered = shutil.which("pi")
        if discovered:
            return Path(discovered)
        raise AgentBinaryNotFoundError("pi executable not found in PATH")
