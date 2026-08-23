from __future__ import annotations

import os
import secrets
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import CommonTestConfig, TestCaseInputs
from tests.v2.common.core.loopback_port import resolve_test_port


@dataclass(frozen=True)
class AlertForwardingRegressionConfig(CommonTestConfig):
    operator_config: Path
    web_port: int
    subscriber_port: int
    subscriber_token: str
    alert_timeout_seconds: float
    negative_window_seconds: float

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "AlertForwardingRegressionConfig":
        common = CommonTestConfig.from_environment(inputs, "ALERT_FORWARDING")
        alert_timeout = float(
            os.environ.get("ALERT_FORWARDING_E2E_ALERT_TIMEOUT_SECONDS", "20")
        )
        negative_window = float(
            os.environ.get("ALERT_FORWARDING_E2E_NEGATIVE_WINDOW_SECONDS", "2")
        )
        if alert_timeout <= 0 or negative_window <= 0:
            raise ValueError("alert forwarding E2E timeouts must be positive")
        return cls(
            **common.as_kwargs(),
            operator_config=inputs.work_dir / "actraild.conf",
            web_port=resolve_test_port(
                "ALERT_FORWARDING_E2E_WEB_PORT",
                attempts=common.drain_attempts,
                connect_timeout_seconds=common.drain_interval_seconds,
            ),
            subscriber_port=resolve_test_port(
                "ALERT_FORWARDING_E2E_SUBSCRIBER_PORT",
                attempts=common.drain_attempts,
                connect_timeout_seconds=common.drain_interval_seconds,
            ),
            subscriber_token="e2e-" + secrets.token_urlsafe(24),
            alert_timeout_seconds=alert_timeout,
            negative_window_seconds=negative_window,
        )
