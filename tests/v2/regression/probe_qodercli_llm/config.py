from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.config import CommonTestConfig


@dataclass(frozen=True)
class ProbeQoderCliLLMConfig(CommonTestConfig):
    @classmethod
    def from_environment(
        cls,
        repo: Path,
        bin_dir: Path,
    ) -> "ProbeQoderCliLLMConfig":
        common = CommonTestConfig.from_environment(repo, bin_dir, "QODERCLI")
        return cls(**common.as_kwargs())
