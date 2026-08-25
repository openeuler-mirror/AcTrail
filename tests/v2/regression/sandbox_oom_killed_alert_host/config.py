from __future__ import annotations

import os
from dataclasses import dataclass

from tests.v2.common.core import CommonTestConfig, TestCaseInputs


@dataclass(frozen=True)
class SandboxOomKilledAlertHostConfig(CommonTestConfig):
    vsock_port: int
    ready_timeout_seconds: int
    runtime_timeout_seconds: int
    root_discovery_settle_seconds: float

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "SandboxOomKilledAlertHostConfig":
        common = CommonTestConfig.from_environment(
            inputs,
            "SANDBOX_OOM_KILLED_ALERT_HOST",
        )
        prefix = "SANDBOX_OOM_KILLED_ALERT_HOST_E2E_"
        port = int(os.environ.get(f"{prefix}VSOCK_PORT", "44183"))
        if port < 1027 or port > 65535:
            raise ValueError(f"{prefix}VSOCK_PORT must be 1027..65535")
        ready_timeout = int(
            os.environ.get(f"{prefix}READY_TIMEOUT_SECONDS", "90")
        )
        runtime_timeout = int(
            os.environ.get(f"{prefix}RUNTIME_TIMEOUT_SECONDS", "300")
        )
        settle = float(
            os.environ.get(f"{prefix}ROOT_DISCOVERY_SETTLE_SECONDS", "3")
        )
        if ready_timeout <= 0 or runtime_timeout <= settle or settle <= 0:
            raise ValueError(
                "focused host OOM alert timeouts must be positive and runtime "
                "must exceed root discovery settle time"
            )
        return cls(
            **common.as_kwargs(),
            vsock_port=port,
            ready_timeout_seconds=ready_timeout,
            runtime_timeout_seconds=runtime_timeout,
            root_discovery_settle_seconds=settle,
        )
