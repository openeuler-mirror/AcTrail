"""Runtime defaults for this regression case. Durations are seconds."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ContainerAutoConstants:
    docker_info_timeout_seconds: int = 15
    timeout_seconds: int = 1200
    cleanup_grace_seconds: int = 30


CASE_CONSTANTS = ContainerAutoConstants()
