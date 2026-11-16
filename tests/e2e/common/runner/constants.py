from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class RunnerDefaults:
    bin_dir: Path = Path("target/release")
    log_dir: Path = Path("/tmp/actrail-regression/logs")
    work_root: Path = Path("/tmp/actrail-regression")
    lock_path: Path = Path("/run/lock/actrail-v2-regression.lock")
    lock_timeout_seconds: float = 900.0
    lock_poll_seconds: float = 1.0
    cleanup: bool = True
    fail_fast: bool = False
    environment_probe_timeout_seconds: float = 10.0
    environment_probe_agents: tuple[str, ...] = ("codex", "claude", "xiaoo")


DEFAULTS = RunnerDefaults()
