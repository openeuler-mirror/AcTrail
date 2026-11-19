from __future__ import annotations

from dataclasses import dataclass

from tests.e2e.common.core import CommonTestConfig, TestRunDirectories


@dataclass(frozen=True)
class ProbePiLLMConfig(CommonTestConfig):
    @classmethod
    def from_environment(cls, directories: TestRunDirectories) -> "ProbePiLLMConfig":
        common = CommonTestConfig.from_environment(directories, "PI")
        return cls(**common.as_kwargs())
