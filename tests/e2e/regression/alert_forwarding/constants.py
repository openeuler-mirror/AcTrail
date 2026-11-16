"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass, field
from pathlib import Path


@dataclass(frozen=True)
class AlertForwardingConstants:
    plugin_directory: Path = field(default_factory=lambda: Path.home() / ".actrail/plugins")
    alert_timeout_seconds: int = 20
    negative_window_seconds: int = 2
    consecutive_failure_threshold: int = 1
    cooldown_seconds: int = 1
    marker_random_bytes: int = 6
    brief_diagnostic_tail_characters: int = 1000
    command_diagnostic_tail_characters: int = 2000
    launch_diagnostic_tail_characters: int = 3000
    token_random_bytes: int = 24
    loopback_host: str = '127.0.0.1'


CASE_CONSTANTS = AlertForwardingConstants()
