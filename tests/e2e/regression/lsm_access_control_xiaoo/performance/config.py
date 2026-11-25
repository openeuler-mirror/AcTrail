"""Environment configuration for registered real-agent benchmarks."""
from dataclasses import dataclass
import os
from tests.e2e.common.core import TestRunDirectories
from pathlib import Path

from .constants import CASE_CONSTANTS as C


@dataclass
class PerformanceConfig:
    bin_dir: Path
    output: Path
    xiaoo: Path | None
    count: int
    rounds: int
    warmup: int
    timeout: float
    command_timeout: int
    launch_timeout: int
    policy_source: str
    backends: list[str]
    order: list[str]
    peer_poll_seconds: float
    backlog: int
    keep_work: bool = False

    @classmethod
    def from_environment(cls, directories: TestRunDirectories, prefix):
        def value(name, default):
            return os.environ.get(prefix + name, str(default))
        xiaoo = os.environ.get(prefix + 'XIAOO')
        config = cls(
            directories.bin_dir, directories.work_dir / 'measurements.json',
            Path(xiaoo) if xiaoo else None,
            int(value('COUNT', C.count)), int(value('ROUNDS', C.rounds)),
            int(value('WARMUP', C.warmup)), float(value('TIMEOUT', C.timeout)),
            int(value('COMMAND_TIMEOUT', C.command_timeout)),
            int(value('LAUNCH_TIMEOUT', C.launch_timeout)),
            value('POLICY_SOURCE', C.policy_source),
            value('BACKENDS', ' '.join(C.backends)).split(),
            value('ORDER', 'off lsm lsm off').split(),
            float(value('PEER_POLL_SECONDS', C.peer_poll_seconds)),
            int(value('BACKLOG', C.backlog)),
        )
        if min(config.count, config.warmup, config.timeout, config.command_timeout,
               config.launch_timeout, config.peer_poll_seconds, config.backlog) <= 0 or config.rounds < 3:
            raise ValueError('benchmark values must be positive and rounds at least three')
        if config.policy_source not in ('static', 'wasm') or sorted(config.backends) != ['lsm', 'seccomp']:
            raise ValueError('invalid benchmark policy source or backends')
        if not config.order or any(item not in ('off', 'empty', 'lsm') for item in config.order):
            raise ValueError('invalid retained benchmark order')
        return config
