from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import CommonTestConfig, TestCaseInputs
from tests.v2.common.core.loopback_port import resolve_test_port
from tests.v2.common.llm_trajectory.config import TrajectoryTestConfig

from .constants import CASE_CONSTANTS


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
                str(CASE_CONSTANTS.trace_random_bytes),
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
                CASE_CONSTANTS.web_host,
            ),
            web_port=resolve_test_port(
                "PROJECT_SUBAGENT_TRAJECTORY_E2E_WEB_PORT",
                attempts=common.drain_attempts,
                connect_timeout_seconds=common.drain_interval_seconds,
            ),
            plugin_package=CASE_CONSTANTS.plugin_package,
            plugin_instance=CASE_CONSTANTS.plugin_instance,
            request_content_max_bytes=int(
                os.environ.get(
                    "PROJECT_SUBAGENT_TRAJECTORY_E2E_REQUEST_CONTENT_MAX_BYTES",
                    str(CASE_CONSTANTS.request_content_max_bytes),
                )
            ),
            agent_binary=agent_binary,
            trace_random_bytes=trace_random_bytes,
        )
