from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.config import CommonTestConfig


@dataclass(frozen=True)
class ProbePiLLMConfig(CommonTestConfig):
    @classmethod
    def from_environment(cls, repo: Path, bin_dir: Path) -> "ProbePiLLMConfig":
        common = CommonTestConfig.from_environment(repo, bin_dir, "PI")
        return cls(**common.as_kwargs())
