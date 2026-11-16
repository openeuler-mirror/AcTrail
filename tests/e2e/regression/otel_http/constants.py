"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class OtelHttpConstants:
    max_turns: int = 1
    marker_random_bytes: int = 8
    plugin_package: str = 'otel-http'
    plugin_instance: str = 'v2.otel-http'
    web_host: str = '127.0.0.1'
    configure_buffered_export_attribute_mode: str = 'metadata-only'
    max_request_bytes: int = 16777216
    init_host: str = '127.0.0.1'
    init_port: int = 0
    stop_timeout: int = 10
    launch_diagnostic_tail_characters: int = 4000
    command_diagnostic_tail_characters: int = 2000
    batch_max_spans: int = 4096
    batch_timeout_ms: int = 60000
    connect_timeout_ms: int = 250
    request_timeout_ms: int = 500
    shutdown_flush_deadline_ms: int = 3000
    queue_capacity: int = 128
    retry_max_attempts: int = 1
    retry_backoff_ms: int = 1
    replacement_batch_timeout_ms: int = 59000


CASE_CONSTANTS = OtelHttpConstants()
