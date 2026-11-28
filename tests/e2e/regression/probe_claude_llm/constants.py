"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ProbeClaudeLlmConstants:
    prompt: str = "What causes the phases of the Moon?"
    launch_diagnostic_tail_characters: int = 4000


CASE_CONSTANTS = ProbeClaudeLlmConstants()
