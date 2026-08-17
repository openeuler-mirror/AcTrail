from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import CommonTestConfig, TestCaseInputs
from tests.v2.common.core.loopback_port import resolve_test_port
from tests.v2.common.testing_env import default_claude_model
from tests.v2.regression.llm_trajectory.config import LlmTrajectoryConfig


@dataclass(frozen=True)
class ClaudeTrajectoryConfig(LlmTrajectoryConfig):
    model: str

    @classmethod
    def from_environment(cls, inputs: TestCaseInputs) -> "ClaudeTrajectoryConfig":
        common = CommonTestConfig.from_environment(inputs, "CLAUDE_TRAJECTORY")
        configured_operator = os.environ.get(
            "CLAUDE_TRAJECTORY_E2E_OPERATOR_CONFIG"
        )
        return cls(
            **common.as_kwargs(),
            operator_config=(
                Path(configured_operator)
                if configured_operator
                else inputs.work_dir / "actraild.conf"
            ),
            web_host=os.environ.get(
                "CLAUDE_TRAJECTORY_E2E_WEB_HOST",
                "127.0.0.1",
            ),
            web_port=resolve_test_port(
                "CLAUDE_TRAJECTORY_E2E_WEB_PORT",
                attempts=common.drain_attempts,
                connect_timeout_seconds=common.drain_interval_seconds,
            ),
            plugin_package="otel-http",
            plugin_instance="v2.claude-trajectory-otel-http",
            request_content_max_bytes=int(
                os.environ.get(
                    "CLAUDE_TRAJECTORY_E2E_REQUEST_CONTENT_MAX_BYTES",
                    str(16 * 1024 * 1024),
                )
            ),
            model=default_claude_model(),
        )
