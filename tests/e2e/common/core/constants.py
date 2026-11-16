from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class CommonTestDefaults:
    command_timeout_seconds: int = 30
    launch_timeout_seconds: int = 180
    drain_attempts: int = 30
    drain_interval_seconds: float = 1.0
    color_mode: str = "auto"
    loopback_host: str = "127.0.0.1"
    port_allocation_attempts: int = 30
    port_connect_timeout_seconds: float = 1.0


DEFAULTS = CommonTestDefaults()
