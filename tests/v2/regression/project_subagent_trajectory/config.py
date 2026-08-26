from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import CommonTestConfig, TestCaseInputs
from tests.v2.common.core.loopback_port import resolve_test_port
from tests.v2.common.llm_trajectory.config import TrajectoryTestConfig


@dataclass(frozen=True)
class ProjectSubagentTrajectoryConfig(TrajectoryTestConfig):
    agent_binary: str | None
    trace_random_bytes: int

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "ProjectSubagentTrajectoryConfig":
        common = CommonTestConfig.from_environment(
            inputs,
            "PROJECT_SUBAGENT_TRAJECTORY",
        )
        configured_operator = os.environ.get(
            "PROJECT_SUBAGENT_TRAJECTORY_E2E_OPERATOR_CONFIG"
        )
        configured_agent = os.environ.get(
            "PROJECT_SUBAGENT_TRAJECTORY_E2E_AGENT_BINARY"
        )
        agent_binary = (
            configured_agent.strip().lower() if configured_agent else None
        )
        if agent_binary is not None and agent_binary not in {
            "opencode",
            "claude",
            "xiaoo",
        }:
            raise ValueError(
                "PROJECT_SUBAGENT_TRAJECTORY_E2E_AGENT_BINARY must be "
                "opencode, claude, or xiaoo"
            )
        trace_random_bytes = int(
            os.environ.get(
                "PROJECT_SUBAGENT_TRAJECTORY_E2E_TRACE_RANDOM_BYTES",
                "3",
            )
        )
        if not 3 <= trace_random_bytes <= 8:
            raise ValueError(
                "PROJECT_SUBAGENT_TRAJECTORY_E2E_TRACE_RANDOM_BYTES must be "
                "between 3 and 8"
            )
        return cls(
            **common.as_kwargs(),
            operator_config=(
                Path(configured_operator)
                if configured_operator
                else inputs.work_dir / "actraild.conf"
            ),
            web_host=os.environ.get(
                "PROJECT_SUBAGENT_TRAJECTORY_E2E_WEB_HOST",
                "127.0.0.1",
            ),
            web_port=resolve_test_port(
                "PROJECT_SUBAGENT_TRAJECTORY_E2E_WEB_PORT",
                attempts=common.drain_attempts,
                connect_timeout_seconds=common.drain_interval_seconds,
            ),
            plugin_package="otel-http",
            plugin_instance="v2.project-subagent-trajectory-otel-http",
            request_content_max_bytes=int(
                os.environ.get(
                    "PROJECT_SUBAGENT_TRAJECTORY_E2E_REQUEST_CONTENT_MAX_BYTES",
                    str(16 * 1024 * 1024),
                )
            ),
            agent_binary=agent_binary,
            trace_random_bytes=trace_random_bytes,
        )
