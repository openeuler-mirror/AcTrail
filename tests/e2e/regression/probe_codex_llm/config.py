from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.e2e.common.core import CommonTestConfig, TestRunDirectories
from tests.e2e.common.agents import AgentFactory

from .constants import CASE_CONSTANTS


@dataclass(frozen=True)
class ProbeCodexLLMConfig(CommonTestConfig):
    model: str
    reasoning_effort: str
    codex_binary: Path | None

    @classmethod
    def from_environment(
        cls,
        directories: TestRunDirectories,
    ) -> "ProbeCodexLLMConfig":
        common = CommonTestConfig.from_environment(directories, "CODEX")
        configured_codex = os.environ.get("CODEX_E2E_BINARY")
        return cls(
            **common.as_kwargs(),
            model=(
                AgentFactory.codex.get_default_llm_model()
                or CASE_CONSTANTS.fallback_model
            ),
            reasoning_effort=os.environ.get("CODEX_E2E_REASONING_EFFORT", CASE_CONSTANTS.reasoning_effort),
            codex_binary=Path(configured_codex) if configured_codex else None,
        )
