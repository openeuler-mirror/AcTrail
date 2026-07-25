from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass(frozen=True)
class CommonTestConfig:
    repo: Path
    bin_dir: Path
    command_timeout_seconds: int
    launch_timeout_seconds: int
    drain_attempts: int
    drain_interval_seconds: float

    @classmethod
    def from_environment(
        cls,
        repo: Path,
        bin_dir: Path,
        environment_prefix: str,
    ) -> "CommonTestConfig":
        def value(name: str, default: str) -> str:
            return os.environ.get(f"{environment_prefix}_E2E_{name}", default)

        return cls(
            repo=repo,
            bin_dir=bin_dir,
            command_timeout_seconds=int(value("COMMAND_TIMEOUT_SECONDS", "30")),
            launch_timeout_seconds=int(value("LAUNCH_TIMEOUT_SECONDS", "180")),
            drain_attempts=int(value("DRAIN_ATTEMPTS", "30")),
            drain_interval_seconds=float(value("DRAIN_INTERVAL_SECONDS", "1")),
        )

    def as_kwargs(self) -> dict[str, Any]:
        return {
            "repo": self.repo,
            "bin_dir": self.bin_dir,
            "command_timeout_seconds": self.command_timeout_seconds,
            "launch_timeout_seconds": self.launch_timeout_seconds,
            "drain_attempts": self.drain_attempts,
            "drain_interval_seconds": self.drain_interval_seconds,
        }
