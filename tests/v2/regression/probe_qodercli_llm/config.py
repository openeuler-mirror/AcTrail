from __future__ import annotations

from dataclasses import dataclass

from tests.v2.common.config import CommonTestConfig, TestCaseInputs


@dataclass(frozen=True)
class ProbeQoderCliLLMConfig(CommonTestConfig):
    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "ProbeQoderCliLLMConfig":
        common = CommonTestConfig.from_environment(inputs, "QODERCLI")
        return cls(**common.as_kwargs())
