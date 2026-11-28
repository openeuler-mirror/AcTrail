from __future__ import annotations

from dataclasses import dataclass

from tests.e2e.common.core import CommonTestConfig, TestRunDirectories


@dataclass(frozen=True)
class ProbeQoderCliLLMConfig(CommonTestConfig):
    @classmethod
    def from_environment(
        cls,
        directories: TestRunDirectories,
    ) -> "ProbeQoderCliLLMConfig":
        common = CommonTestConfig.from_environment(directories, "QODERCLI")
        return cls(**common.as_kwargs())
