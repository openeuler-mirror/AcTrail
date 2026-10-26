"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ProbeQodercliLlmConstants:
    launch_diagnostic_tail_characters: int = 4000


CASE_CONSTANTS = ProbeQodercliLlmConstants()
