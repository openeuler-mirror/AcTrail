from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import CommonTestConfig, TestCaseInputs
from tests.v2.common.testing_env import default_claude_model


@dataclass(frozen=True)
class ProbeClaudeLLMConfig(CommonTestConfig):
    model: str
    claude_binary: Path | None

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "ProbeClaudeLLMConfig":
        common = CommonTestConfig.from_environment(inputs, "CLAUDE")
        configured_binary = os.environ.get("CLAUDE_E2E_BINARY")
        return cls(
            **common.as_kwargs(),
            model=default_claude_model(),
            claude_binary=Path(configured_binary) if configured_binary else None,
        )
