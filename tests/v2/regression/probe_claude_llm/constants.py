"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ProbeClaudeLlmConstants:
    marker_random_bytes: int = 5
    launch_diagnostic_tail_characters: int = 4000


CASE_CONSTANTS = ProbeClaudeLlmConstants()
