from __future__ import annotations

from dataclasses import dataclass

from tests.v2.common.config import CommonTestConfig, TestCaseInputs


@dataclass(frozen=True)
class ProbePiLLMConfig(CommonTestConfig):
    @classmethod
    def from_environment(cls, inputs: TestCaseInputs) -> "ProbePiLLMConfig":
        common = CommonTestConfig.from_environment(inputs, "PI")
        return cls(**common.as_kwargs())
