"""Default runtime parameters for container resource acceptance."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ResourceMetricsContainerConstants:
    expect_rss: bool = False
    memory_accounting_jitter_bytes: int = 4 * 1024**2
    workload_seconds: int = 120
    initial_sample_count: int = 3
    recovery_sample_count: int = 3
    second_trace_sample_count: int = 2
    process_exit_timeout_seconds: int = 10
    container_stop_seconds: int = 1
    binary_relative_directory: str = "target/release"
    work_directory_prefix: str = "actrail-container-acceptance-"
    diagnostic_tail_characters: int = 8000


CASE_CONSTANTS = ResourceMetricsContainerConstants()
