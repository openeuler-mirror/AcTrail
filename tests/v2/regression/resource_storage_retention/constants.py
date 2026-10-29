"""Runtime parameters for the optional resource retention acceptance."""
from dataclasses import dataclass


@dataclass(frozen=True)
class ResourceStorageRetentionConstants:
    expired_per_table: int = 200_000
    batch_size: int = 32
    max_age_seconds: int = 86400
    sweep_interval_seconds: int = 3600
    sample_interval_ms: int = 50
    poll_seconds: float = 0.05
    cleanup_timeout_seconds: int = 30
    command_timeout_seconds: int = 30
    control_request_timeout_seconds: int = 1
    stop_timeout_seconds: int = 10
    kill_timeout_seconds: int = 5


CASE_CONSTANTS = ResourceStorageRetentionConstants()
