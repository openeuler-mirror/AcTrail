from __future__ import annotations

import os
import secrets
from dataclasses import dataclass
from pathlib import Path

from tests.e2e.common.core import CommonTestConfig, TestRunDirectories
from tests.e2e.common.core.loopback_port import resolve_test_port

from .constants import CASE_CONSTANTS


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
        directories: TestRunDirectories,
    ) -> "AlertForwardingRegressionConfig":
        common = CommonTestConfig.from_environment(directories, "ALERT_FORWARDING")
        alert_timeout = float(
            os.environ.get("ALERT_FORWARDING_E2E_ALERT_TIMEOUT_SECONDS", str(CASE_CONSTANTS.alert_timeout_seconds))
        )
        negative_window = float(
            os.environ.get("ALERT_FORWARDING_E2E_NEGATIVE_WINDOW_SECONDS", str(CASE_CONSTANTS.negative_window_seconds))
        )
        if alert_timeout <= 0 or negative_window <= 0:
            raise ValueError("alert forwarding E2E timeouts must be positive")
        return cls(
            **common.as_kwargs(),
            operator_config=directories.work_dir / "actraild.conf",
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
            subscriber_token="e2e-" + secrets.token_urlsafe(CASE_CONSTANTS.token_random_bytes),
            alert_timeout_seconds=alert_timeout,
            negative_window_seconds=negative_window,
        )
