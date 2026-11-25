"""Defaults shared by process-owning agent adapters."""
from dataclasses import dataclass


@dataclass(frozen=True)
class AgentDefaults:
    shutdown_timeout_seconds: float = 5.0


DEFAULTS = AgentDefaults()
