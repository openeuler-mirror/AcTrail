from __future__ import annotations

import os
from dataclasses import dataclass

from tests.v2.common.core import CommonTestConfig, TestCaseInputs

from .constants import CASE_CONSTANTS


@dataclass(frozen=True)
class SandboxResourceAlertHostConfig(CommonTestConfig):
    vsock_port: int
    ready_timeout_seconds: int
    runtime_timeout_seconds: int
    root_discovery_settle_seconds: float

    @staticmethod
    def xiaoo_config(provider_port: int) -> str:
        return f"""[llm]
provider = "{CASE_CONSTANTS.llm_provider}"
model = "{CASE_CONSTANTS.llm_model}"
api_key_env = "ACTRAIL_VIRTUAL_XIAOO_API_KEY"
api_base = "http://{CASE_CONSTANTS.provider_host}:{provider_port}"
max_tokens = {CASE_CONSTANTS.llm_max_tokens}
context_window = {CASE_CONSTANTS.llm_context_window}
reasoning_effort = "{CASE_CONSTANTS.llm_reasoning_effort}"
"""

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "SandboxResourceAlertHostConfig":
        common = CommonTestConfig.from_environment(
            inputs,
            "SANDBOX_RESOURCE_ALERT_HOST",
        )
        prefix = "SANDBOX_RESOURCE_ALERT_HOST_E2E_"
        port = int(os.environ.get(f"{prefix}VSOCK_PORT", str(CASE_CONSTANTS.vsock_port)))
        if port < 1027 or port > 65535:
            raise ValueError(f"{prefix}VSOCK_PORT must be 1027..65535")
        ready_timeout = int(
            os.environ.get(f"{prefix}READY_TIMEOUT_SECONDS", str(CASE_CONSTANTS.ready_timeout_seconds))
        )
        runtime_timeout = int(
            os.environ.get(f"{prefix}RUNTIME_TIMEOUT_SECONDS", str(CASE_CONSTANTS.runtime_timeout_seconds))
        )
        settle = float(
            os.environ.get(f"{prefix}ROOT_DISCOVERY_SETTLE_SECONDS", str(CASE_CONSTANTS.root_discovery_settle_seconds))
        )
        if ready_timeout <= 0 or runtime_timeout <= 0 or settle <= 0:
            raise ValueError("host sandbox alert timeouts must be positive")
        return cls(
            **common.as_kwargs(),
            vsock_port=port,
            ready_timeout_seconds=ready_timeout,
            runtime_timeout_seconds=runtime_timeout,
            root_discovery_settle_seconds=settle,
        )
