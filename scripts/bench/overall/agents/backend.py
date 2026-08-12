"""Shared agent backend definition."""

from __future__ import annotations

import shutil
from dataclasses import dataclass
from pathlib import Path
from typing import Callable


@dataclass(frozen=True)
class AgentBackend:
    name: str
    binary: Path | None
    command: Callable[[int, str, int], list[str]]
    prepare: Callable[[Path, int], None] | None = None
    run_cwd: Callable[[Path], Path] | None = None
    case_timeout_seconds: float = 900.0

    def prepare_workdir(self, work_dir: Path, replay_port: int) -> None:
        if self.prepare is not None:
            self.prepare(work_dir, replay_port)

    def working_directory(self, work_dir: Path) -> Path | None:
        """Working directory for the command, or None for the repo root."""
        if self.run_cwd is not None:
            return self.run_cwd(work_dir)
        return None


def resolve_binary(configured: str | None, default_name: str) -> Path | None:
    if configured:
        return Path(configured)
    found = shutil.which(default_name)
    return Path(found) if found else None
