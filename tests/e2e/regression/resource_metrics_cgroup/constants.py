"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ResourceMetricsCgroupConstants:
    assert_final_timed_out: bool = False
    wait_until_timeout: int = 8
    run_exact_case_child_seconds: float = 0.8
    run_exact_case_parent_seconds: float = 0.2
    run_exact_case_timeout: int = 8
    run_restart_case_child_seconds: float = 4.0
    run_restart_case_parent_seconds: float = 3.5
    run_restart_case_timeout: int = 8
    run_timeout_case_child_seconds: float = 3.0
    run_timeout_case_parent_seconds: float = 0.2
    run_timeout_case_timeout: int = 6
    run_lost_scope_case_child_seconds: float = 1.0
    run_lost_scope_case_parent_seconds: float = 0.8
    run_lost_scope_case_timeout: int = 8
    recovery_child_seconds: float = 0.8
    recovery_parent_seconds: float = 0.2
    launch_timeout: int = 8
    wait_for_trace_timeout: int = 10
    command_timeout: int = 30
    limit: int = 10000
    wait_until_poll_seconds: float = 0.05
    parent_memory_bytes: int = 25165824
    child_memory_bytes: int = 41943040
    memory_touch_stride_bytes: int = 4096
    minimum_peak_bytes: int = 50331648
    minimum_systemd_version: int = 254
    service_restart_milliseconds: int = 200
    web_ready_timeout: int = 10
    web_request_timeout: int = 10
    web_stop_timeout: int = 5
    web_poll_seconds: float = 0.05
    resource_api_max_points: int = 8


CASE_CONSTANTS = ResourceMetricsCgroupConstants()
