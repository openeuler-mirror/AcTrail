from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.e2e.common.core import CommonTestConfig, TestRunDirectories
from tests.e2e.common.core.loopback_port import resolve_test_port

from .constants import CASE_CONSTANTS


@dataclass(frozen=True)
class OtelJsonlActionFilterConfig(CommonTestConfig):
    operator_config: Path
    web_host: str
    web_port: int
    plugin_package: str
    plugin_instance: str
    agent_max_turns: int

    @classmethod
    def from_environment(
        cls,
        directories: TestRunDirectories,
    ) -> "OtelJsonlActionFilterConfig":
        common = CommonTestConfig.from_environment(
            directories,
            "OTEL_JSONL_ACTION_FILTER",
        )
        configured_operator = os.environ.get(
            "OTEL_JSONL_ACTION_FILTER_E2E_OPERATOR_CONFIG"
        )
        agent_max_turns = int(os.environ.get(
            "OTEL_JSONL_ACTION_FILTER_E2E_AGENT_MAX_TURNS",
            CASE_CONSTANTS.agent_max_turns,
        ))
        if agent_max_turns < 1:
            raise ValueError("OTEL_JSONL_ACTION_FILTER_E2E_AGENT_MAX_TURNS must be positive")
        return cls(
            **common.as_kwargs(),
            operator_config=(
                Path(configured_operator)
                if configured_operator
                else directories.work_dir / "actraild.conf"
            ),
            web_host=os.environ.get(
                "OTEL_JSONL_ACTION_FILTER_E2E_WEB_HOST",
                CASE_CONSTANTS.web_host,
            ),
            web_port=resolve_test_port(
                "OTEL_JSONL_ACTION_FILTER_E2E_WEB_PORT",
                attempts=common.drain_attempts,
                connect_timeout_seconds=common.drain_interval_seconds,
            ),
            plugin_package=CASE_CONSTANTS.plugin_package,
            plugin_instance=CASE_CONSTANTS.plugin_instance,
            agent_max_turns=agent_max_turns,
        )
