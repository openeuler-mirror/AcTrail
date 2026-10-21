"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ProbeCodexLlmConstants:
    reasoning_effort: str = 'low'
    marker_random_bytes: int = 5
    launch_diagnostic_tail_characters: int = 4000
    fallback_model: str = 'gpt-5.5'


CASE_CONSTANTS = ProbeCodexLlmConstants()
