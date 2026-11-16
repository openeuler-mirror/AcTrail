"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class OtelJsonlActionFilterConstants:
    plugin_package: str = 'otel-jsonl'
    plugin_instance: str = 'v2.otel-jsonl-action-filter'
    web_host: str = '127.0.0.1'
    delay_next_json_rpc_responses_count: int = 1
    max_request_bytes: int = 16777216
    init_host: str = '127.0.0.1'
    init_port: int = 0
    delay_next_responses_count: int = 1
    send_json_close_connection: bool = False
    send_json_ignore_disconnect: bool = False
    poll_interval_seconds: float = 0.1
    host: str = '127.0.0.1'
    port: int = 0
    method: str = 'otel.export'
    fail_next: int = 0
    delay_next_ms: int = 0
    stop_timeout: int = 10
    marker_random_bytes: int = 6
    connect_timeout_ms: int = 250
    request_timeout_ms: int = 500
    response_body_max_bytes: int = 65536
    inject_retryable_failure: bool = False
    launch_diagnostic_tail_characters: int = 4000
    command_diagnostic_tail_characters: int = 2000
    max_attempts: int = 3
    retry_backoff_ms: int = 10
    injected_response_delay_seconds: float = 0.75


CASE_CONSTANTS = OtelJsonlActionFilterConstants()
