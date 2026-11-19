"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ProbeCodexLlmConstants:
    reasoning_effort: str = 'low'
    prompt: str = "Why do the phases of the Moon change during the month?"
    launch_diagnostic_tail_characters: int = 4000
    fallback_model: str = 'gpt-5.5'


CASE_CONSTANTS = ProbeCodexLlmConstants()
