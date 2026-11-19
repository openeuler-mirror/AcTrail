from __future__ import annotations

from dataclasses import dataclass

from tests.e2e.common.core import CommonTestConfig, TestRunDirectories


@dataclass(frozen=True)
class ProbeDcodeLLMConfig(CommonTestConfig):
    @classmethod
    def from_environment(cls, directories: TestRunDirectories) -> "ProbeDcodeLLMConfig":
        common = CommonTestConfig.from_environment(directories, "DCODE")
        return cls(**common.as_kwargs())
