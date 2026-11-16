from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class RuntimeTimingDefaults:
    shutdown_wait_ms: int = 150100
    supervision_poll_interval_ms: int = 100
    finalization_shutdown_drain_timeout_ms: int = 30000
    post_trace_broker_reply_timeout_ms: int = 5000
    post_trace_shutdown_drain_timeout_ms: int = 30000
    plugin_alert_drain_timeout_ms: int = 30000


DEFAULTS = RuntimeTimingDefaults()
