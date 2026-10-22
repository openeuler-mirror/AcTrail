"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class SandboxResourceAlertHostConstants:
    vsock_port: int = 44182
    ready_timeout_seconds: int = 90
    runtime_timeout_seconds: int = 300
    root_discovery_settle_seconds: int = 3
    read_threshold_bytes: int = 786432
    write_threshold_bytes: int = 2097152
    verify_disconnect_reconnect_grace_seconds: int = 3
    require_alive_poll_seconds: float = 0.1
    verify_dynamic_memory_policy_poll_seconds: float = 0.1
    wait_memory_baseline_poll_seconds: float = 0.1
    wait_reconnected_resource_alert_poll_seconds: float = 0.1
    wait_all_alerts_poll_seconds: float = 0.1
    wait_path_poll_seconds: float = 0.05
    require_alive_timeout: int = 1
    start_alert_path_memory_available_bytes: int = 18446744073709551615
    updated_memory_threshold_bytes: int = 1
    start_gateway_grace_seconds: int = 1
    wait_path_timeout: int = 1
    marker_random_bytes: int = 8
    run_grace_seconds: int = 3
    wait_tcp_timeout: float = 0.2
    wait_tcp_poll_seconds: float = 0.05
    launch_diagnostic_tail_characters: int = 4000
    observation_window_poll_count: int = 2
    observation_window_slack_seconds: float = 0.25
    cpu_usage_basis_points: int = 10000
    task_file_bytes: int = 2097152
    oom_memory_limit_bytes: int = 33554432
    provider_host: str = '127.0.0.1'
    api_key: str = 'actrail-host-local-key'
    provider_reasoning_tokens: int = 3
    workload_read_bytes: int = 1048576
    workload_write_block_bytes: int = 1048576
    workload_write_blocks: int = 4
    workload_poll_seconds: int = 1
    workload_max_turns: int = 4
    llm_provider: str = 'deepseek'
    llm_model: str = 'deepseek-chat'
    llm_max_tokens: int = 128
    llm_context_window: int = 32768
    llm_reasoning_effort: str = 'off'
    loopback_host: str = '127.0.0.1'


CASE_CONSTANTS = SandboxResourceAlertHostConstants()
