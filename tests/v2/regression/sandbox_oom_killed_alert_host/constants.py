"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class SandboxOomKilledAlertHostConstants:
    vsock_port: int = 44183
    ready_timeout_seconds: int = 90
    runtime_timeout_seconds: int = 300
    root_discovery_settle_seconds: int = 3
    require_alive_poll_seconds: float = 0.1
    require_alive_timeout: int = 1
    create_alert_path_memory_available_bytes: int = 1
    start_gateway_grace_seconds: int = 1
    marker_random_bytes: int = 8
    run_grace_seconds: int = 3
    wait_tcp_timeout: float = 0.2
    wait_tcp_poll_seconds: float = 0.05
    launch_diagnostic_tail_characters: int = 4000
    observation_window_poll_count: int = 2
    observation_window_slack_seconds: float = 0.25
    cpu_usage_basis_points: int = 10000
    loopback_host: str = '127.0.0.1'


CASE_CONSTANTS = SandboxOomKilledAlertHostConstants()
