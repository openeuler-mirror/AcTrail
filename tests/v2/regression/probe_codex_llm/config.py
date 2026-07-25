from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class ProbeCodexLLMConfig:
    repo: Path
    bin_dir: Path
    model: str
    reasoning_effort: str
    codex_binary: Path | None
    command_timeout_seconds: int
    launch_timeout_seconds: int
    drain_attempts: int
    drain_interval_seconds: float

    @classmethod
    def from_environment(cls, repo: Path, bin_dir: Path) -> "ProbeCodexLLMConfig":
        configured_codex = os.environ.get("CODEX_E2E_BINARY")
        return cls(
            repo=repo,
            bin_dir=bin_dir,
            model=os.environ.get("CODEX_E2E_MODEL", "gpt-5.5"),
            reasoning_effort=os.environ.get("CODEX_E2E_REASONING_EFFORT", "low"),
            codex_binary=Path(configured_codex) if configured_codex else None,
            command_timeout_seconds=int(
                os.environ.get("CODEX_E2E_COMMAND_TIMEOUT_SECONDS", "30")
            ),
            launch_timeout_seconds=int(
                os.environ.get("CODEX_E2E_LAUNCH_TIMEOUT_SECONDS", "180")
            ),
            drain_attempts=int(os.environ.get("CODEX_E2E_DRAIN_ATTEMPTS", "30")),
            drain_interval_seconds=float(
                os.environ.get("CODEX_E2E_DRAIN_INTERVAL_SECONDS", "1")
            ),
        )
