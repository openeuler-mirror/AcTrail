"""Runtime defaults for this regression case. Durations are seconds."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ContainerAgentXiaooConstants:
    docker_info_timeout_seconds: int = 15
    timeout_seconds: int = 900
    cleanup_grace_seconds: int = 30
    image: str = 'ubuntu:24.04'


CASE_CONSTANTS = ContainerAgentXiaooConstants()
