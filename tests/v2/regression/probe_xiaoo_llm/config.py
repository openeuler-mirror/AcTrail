from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.config import CommonTestConfig, TestCaseInputs


@dataclass(frozen=True)
class ProbeXiaooLLMConfig(CommonTestConfig):
    xiaoo_binary: Path | None

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "ProbeXiaooLLMConfig":
        common = CommonTestConfig.from_environment(inputs, "XIAOO")
        configured_binary = os.environ.get("XIAOO_E2E_BINARY")
        return cls(
            **common.as_kwargs(),
            xiaoo_binary=Path(configured_binary) if configured_binary else None,
        )
