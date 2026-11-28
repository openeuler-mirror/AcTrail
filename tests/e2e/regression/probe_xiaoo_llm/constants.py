"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ProbeXiaooLlmConstants:
    max_turns: int = 1
    prompt: str = "Why does the sky look blue during the day?"
    launch_diagnostic_tail_characters: int = 4000


CASE_CONSTANTS = ProbeXiaooLlmConstants()
