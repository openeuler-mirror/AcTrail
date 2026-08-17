from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import CommonTestConfig, TestCaseInputs
from tests.v2.common.core.loopback_port import resolve_test_port


@dataclass(frozen=True)
class LlmTrajectoryConfig(CommonTestConfig):
    operator_config: Path
    web_host: str
    web_port: int
    plugin_package: str
    plugin_instance: str
    request_content_max_bytes: int

    @classmethod
    def from_environment(cls, inputs: TestCaseInputs) -> "LlmTrajectoryConfig":
        common = CommonTestConfig.from_environment(inputs, "LLM_TRAJECTORY")
        configured_operator = os.environ.get("LLM_TRAJECTORY_E2E_OPERATOR_CONFIG")
        return cls(
            **common.as_kwargs(),
            operator_config=(
                Path(configured_operator)
                if configured_operator
                else inputs.work_dir / "actraild.conf"
            ),
            web_host=os.environ.get("LLM_TRAJECTORY_E2E_WEB_HOST", "127.0.0.1"),
            web_port=resolve_test_port(
                "LLM_TRAJECTORY_E2E_WEB_PORT",
                attempts=common.drain_attempts,
                connect_timeout_seconds=common.drain_interval_seconds,
            ),
            plugin_package="otel-http",
            plugin_instance="v2.llm-trajectory-otel-http",
            request_content_max_bytes=int(
                os.environ.get(
                    "LLM_TRAJECTORY_E2E_REQUEST_CONTENT_MAX_BYTES",
                    str(16 * 1024 * 1024),
                )
            ),
        )
