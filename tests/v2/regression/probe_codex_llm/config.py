from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import CommonTestConfig, TestCaseInputs
from tests.v2.common.testing_env import default_codex_model


@dataclass(frozen=True)
class ProbeCodexLLMConfig(CommonTestConfig):
    model: str
    reasoning_effort: str
    codex_binary: Path | None

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "ProbeCodexLLMConfig":
        common = CommonTestConfig.from_environment(inputs, "CODEX")
        configured_codex = os.environ.get("CODEX_E2E_BINARY")
        return cls(
            **common.as_kwargs(),
            model=(
                os.environ.get("CODEX_E2E_MODEL")
                or default_codex_model(inputs.repo)
                or "gpt-5.5"
            ),
            reasoning_effort=os.environ.get("CODEX_E2E_REASONING_EFFORT", "low"),
            codex_binary=Path(configured_codex) if configured_codex else None,
        )
