from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class AlertProxyDefaults:
    subscriber_host: str = "127.0.0.1"
    poll_seconds: float = 0.05
    stop_timeout_seconds: float = 5.0
    connect_timeout_seconds: float = 0.5
    minimum_connect_timeout_seconds: float = 0.05
    reader_join_timeout_seconds: float = 2.0
    minimum_receive_timeout_seconds: float = 0.01
    max_payload_bytes: int = 262_140
    receive_timeout_seconds: float = 3600.0
    forwarding_enabled: bool = False
    forwarding_all_categories: bool = False
    forwarding_categories: tuple[str, ...] = ("consecutive_failure",)
    socket_mode_octal: str = "0600"
    ingress_connection_limit: int = 2
    accept_poll_interval_ms: int = 20
    io_poll_interval_ms: int = 100
    producer_idle_timeout_ms: int = 5000
    max_frame_bytes: int = 262144
    max_trace_id_bytes: int = 128
    max_category_bytes: int = 128
    max_description_bytes: int = 4096
    max_extras_bytes: int = 131072
    worker_thread_stack_bytes: int = 524288
    allow_insecure_remote: bool = False
    listen_backlog: int = 16
    subscriber_connection_limit: int = 8
    max_client_id_bytes: int = 128
    max_request_id_bytes: int = 128
    max_topics: int = 16
    max_topic_bytes: int = 128
    broadcast_queue_capacity: int = 64
    broadcaster_thread_stack_bytes: int = 524288
    queue_capacity: int = 16
    heartbeat_interval_ms: int = 1000
    pong_timeout_ms: int = 500
    peer_idle_timeout_ms: int = 5000


DEFAULTS = AlertProxyDefaults()
