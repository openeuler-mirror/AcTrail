from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .constants import DEFAULTS


@dataclass(frozen=True)
class TestRunDirectories:
    repo: Path
    bin_dir: Path
    work_dir: Path


@dataclass(frozen=True)
class CommonTestConfig:
    repo: Path
    bin_dir: Path
    work_dir: Path
    command_timeout_seconds: int
    launch_timeout_seconds: int
    drain_attempts: int
    drain_interval_seconds: float

    @classmethod
    def from_environment(
        cls,
        directories: TestRunDirectories,
        environment_prefix: str,
    ) -> "CommonTestConfig":
        def value(name: str, default: str) -> str:
            return os.environ.get(f"{environment_prefix}_E2E_{name}", default)

        return cls(
            repo=directories.repo,
            bin_dir=directories.bin_dir,
            work_dir=directories.work_dir,
            command_timeout_seconds=int(value("COMMAND_TIMEOUT_SECONDS", str(DEFAULTS.command_timeout_seconds))),
            launch_timeout_seconds=int(value("LAUNCH_TIMEOUT_SECONDS", str(DEFAULTS.launch_timeout_seconds))),
            drain_attempts=int(value("DRAIN_ATTEMPTS", str(DEFAULTS.drain_attempts))),
            drain_interval_seconds=float(value("DRAIN_INTERVAL_SECONDS", str(DEFAULTS.drain_interval_seconds))),
        )

    def as_kwargs(self) -> dict[str, Any]:
        return {
            "repo": self.repo,
            "bin_dir": self.bin_dir,
            "work_dir": self.work_dir,
            "command_timeout_seconds": self.command_timeout_seconds,
            "launch_timeout_seconds": self.launch_timeout_seconds,
            "drain_attempts": self.drain_attempts,
            "drain_interval_seconds": self.drain_interval_seconds,
        }
