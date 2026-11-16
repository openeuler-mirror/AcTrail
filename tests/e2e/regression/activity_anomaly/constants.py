"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ActivityAnomalyConstants:
    anomaly_hard_limit_bytes: int = 1
    provider_ready_timeout_seconds: int = 15
    alert_timeout_seconds: int = 20
    command_threshold_ms: int = 500
    long_command_seconds: int = 2
    web_host: str = '127.0.0.1'
    bind_port: int = 0
    stop_provider_timeout: int = 10
    wait_for_stable_alerts_poll_seconds: float = 0.2
    trace_random_bytes: int = 4
    response_marker_random_bytes: int = 8
    command_diagnostic_tail_characters: int = 2000
    launch_diagnostic_tail_characters: int = 4000
    io_poll_seconds: float = 0.2
    llm_provider: str = 'deepseek'
    llm_model: str = 'deepseek-chat'
    llm_max_tokens: int = 128
    llm_context_window: int = 32768
    llm_reasoning_effort: str = 'off'
    no_proxy_hosts: str = '127.0.0.1,localhost'
    max_turns: int = 3
    loopback_host: str = '127.0.0.1'


CASE_CONSTANTS = ActivityAnomalyConstants()
