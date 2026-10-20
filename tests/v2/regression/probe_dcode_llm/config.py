from __future__ import annotations

from dataclasses import dataclass

from tests.v2.common.core import CommonTestConfig, TestCaseInputs


@dataclass(frozen=True)
class ProbeDcodeLLMConfig(CommonTestConfig):
    @classmethod
    def from_environment(cls, inputs: TestCaseInputs) -> "ProbeDcodeLLMConfig":
        common = CommonTestConfig.from_environment(inputs, "DCODE")
        return cls(**common.as_kwargs())
