from __future__ import annotations

import os
import shutil
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import CommonTestConfig, TestCaseInputs
from tests.v2.common.core.loopback_port import resolve_test_port


@dataclass(frozen=True)
class ActivityAnomalyConfig(CommonTestConfig):
    operator_config: Path
    operator_config_patch: Path
    xiaoo_binary: Path
    web_host: str
    web_port: int
    provider_ready_timeout_seconds: float
    alert_timeout_seconds: float
    command_threshold_ms: int
    long_command_seconds: int

    @classmethod
    def from_environment(cls, inputs: TestCaseInputs) -> "ActivityAnomalyConfig":
        common = CommonTestConfig.from_environment(inputs, "ACTIVITY_ANOMALY")
        xiaoo = os.environ.get("ACTIVITY_ANOMALY_E2E_XIAOO_BINARY") or shutil.which(
            "xiaoo"
        )
        if xiaoo is None:
            raise RuntimeError("real Xiaoo executable is not available")
        provider_timeout = float(
            os.environ.get(
                "ACTIVITY_ANOMALY_E2E_PROVIDER_READY_TIMEOUT_SECONDS", "15"
            )
        )
        alert_timeout = float(
            os.environ.get("ACTIVITY_ANOMALY_E2E_ALERT_TIMEOUT_SECONDS", "20")
        )
        command_threshold = int(
            os.environ.get("ACTIVITY_ANOMALY_E2E_COMMAND_THRESHOLD_MS", "500")
        )
        long_command_seconds = int(
            os.environ.get("ACTIVITY_ANOMALY_E2E_LONG_COMMAND_SECONDS", "2")
        )
        if provider_timeout <= 0 or alert_timeout <= 0:
            raise RuntimeError("activity-anomaly E2E timeouts must be positive")
        if command_threshold <= 0 or long_command_seconds <= 0:
            raise RuntimeError("activity-anomaly E2E thresholds must be positive")
        if long_command_seconds * 1000 <= command_threshold:
            raise RuntimeError(
                "long command duration must exceed the configured command threshold"
            )
        return cls(
            **common.as_kwargs(),
            operator_config=inputs.work_dir / "actraild.conf",
            operator_config_patch=inputs.work_dir / "actraild.patch.toml",
            xiaoo_binary=Path(xiaoo).resolve(),
            web_host=os.environ.get("ACTIVITY_ANOMALY_E2E_WEB_HOST", "127.0.0.1"),
            web_port=resolve_test_port(
                "ACTIVITY_ANOMALY_E2E_WEB_PORT",
                attempts=common.drain_attempts,
                connect_timeout_seconds=common.drain_interval_seconds,
            ),
            provider_ready_timeout_seconds=provider_timeout,
            alert_timeout_seconds=alert_timeout,
            command_threshold_ms=command_threshold,
            long_command_seconds=long_command_seconds,
        )
