"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ProbeQodercliLlmConstants:
    prompt: str = "Why does a prism split white light into colors?"
    request_protocol_id: str = "qoder-infer"
    launch_diagnostic_tail_characters: int = 4000


CASE_CONSTANTS = ProbeQodercliLlmConstants()
