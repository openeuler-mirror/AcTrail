from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class IsolationDefaults:
    poll_seconds: float = 0.05
    web_stop_timeout_seconds: float = 5.0
    agent_exit_timeout_seconds: float = 1.0
    database_timeout_seconds: float = 1.0
    cgroup_root: Path = Path("/sys/fs/cgroup")
    swaps_path: Path = Path("/proc/swaps")
    precheck_memory_bytes: int = 33554432
    cgroup_cleanup_timeout_seconds: float = 2.0


DEFAULTS = IsolationDefaults()
