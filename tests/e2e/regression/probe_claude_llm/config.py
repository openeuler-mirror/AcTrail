from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.e2e.common.core import CommonTestConfig, TestRunDirectories
from tests.e2e.common.agents import AgentFactory


@dataclass(frozen=True)
class ProbeClaudeLLMConfig(CommonTestConfig):
    model: str
    claude_binary: Path | None

    @classmethod
    def from_environment(
        cls,
        directories: TestRunDirectories,
    ) -> "ProbeClaudeLLMConfig":
        common = CommonTestConfig.from_environment(directories, "CLAUDE")
        configured_binary = os.environ.get("CLAUDE_E2E_BINARY")
        return cls(
            **common.as_kwargs(),
            model=AgentFactory.claude.get_default_llm_model(),
            claude_binary=Path(configured_binary) if configured_binary else None,
        )
