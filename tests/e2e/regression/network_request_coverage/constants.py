"""Defaults for real failed-model and ordinary HTTP workloads."""
from dataclasses import dataclass


@dataclass(frozen=True)
class NetworkRequestCoverageConstants:
    timeout_seconds: int = 180
    max_turns: int = 1
    error_model: str = "actrail-e2e-nonexistent-model"
    prompt: str = "Briefly describe what a text file is."


CASE_CONSTANTS = NetworkRequestCoverageConstants()
