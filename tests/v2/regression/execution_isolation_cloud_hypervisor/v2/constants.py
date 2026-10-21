"""Runtime defaults for this regression case. Durations are seconds."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ExecutionIsolationConstants:
    wait_new_base_socket_poll_seconds: float = 0.05
    wait_listener_socket_poll_seconds: float = 0.05
    ctr_runtime: str = 'io.containerd.kata.v2'
    image_pull_policy: str = 'never'
    vsock_host_cid: int = 2
    vsock_port: int = 43182
    runtime_timeout_seconds: int = 900
    ready_timeout_seconds: int = 90
    root_discovery_settle_seconds: int = 3
    workload_uid: int = 1000
    workload_gid: int = 39000
    image: str = 'docker.io/library/actrail-openeuler-workload:24.09'
    complete_workload_terminate_grace_seconds: int = 5
    stop_processes_grace_seconds: int = 3
    start_vm_gateway_and_connect_grace_seconds: int = 3
    trigger_guest_oom_timeout_seconds: int = 45
    require_alive_poll_seconds: float = 0.1
    wait_path_poll_seconds: float = 0.05
    wait_observation_alerts_poll_seconds: float = 0.05
    wait_resource_baseline_poll_seconds: float = 0.05
    require_alive_timeout_seconds: int = 1
    wait_path_timeout_seconds: int = 1
    wait_tcp_timeout_seconds: float = 0.2
    wait_tcp_poll_seconds: float = 0.05

    vm_root: str = "/run/vc/vm"
    unix_socket_path_limit: int = 107
    vsock_port_minimum: int = 1027
    port_maximum: int = 65535
    vm_hold_seconds: int = 600
    command_timeout_slack_seconds: int = 2
    observer_ready_reserve_seconds: int = 5
    observer_poll_seconds: float = 0.5
    observer_request_timeout_min_ms: int = 100
    observer_request_timeout_max_ms: int = 5000
    observer_request_reserve_seconds: float = 0.5
    diagnostic_tail_lines: int = 80
    resource_alert_cpu_basis_points: int = 1
    resource_alert_io_threshold_bytes: int = 1
    resource_alert_source_capacity: int = 64
    resource_baseline_timeout_seconds: float = 10.0
    provider_port: int = 18098
    provider_host: str = "127.0.0.1"
    local_api_key: str = "actrail-kata-local-key"
    guest_poll_seconds: int = 1
    provider_delay_seconds: float = 1.0
    llm_provider: str = "deepseek"
    llm_model: str = "deepseek-chat"
    llm_max_tokens: int = 128
    llm_context_window: int = 32768
    llm_reasoning_effort: str = "off"

    child_release_poll_seconds: float = 0.05
    oom_memory_limit_bytes: int = 33554432
    oom_poll_seconds: float = 0.1
    oom_kill_attempts: int = 300
    oom_vmstat_attempts: int = 50
    oom_start_poll_seconds: float = 0.01
    oom_allocation_bytes: int = 1024 * 1024
    oom_page_stride_bytes: int = 4096


CASE_CONSTANTS = ExecutionIsolationConstants()
