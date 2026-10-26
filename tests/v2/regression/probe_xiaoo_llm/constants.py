"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ProbeXiaooLlmConstants:
    max_turns: int = 1
    marker_random_bytes: int = 5
    launch_diagnostic_tail_characters: int = 4000


CASE_CONSTANTS = ProbeXiaooLlmConstants()
