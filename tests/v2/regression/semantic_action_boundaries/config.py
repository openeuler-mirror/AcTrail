from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import CommonTestConfig, TestCaseInputs
from tests.v2.common.core.loopback_port import resolve_test_port


@dataclass(frozen=True)
class SemanticActionBoundariesConfig(CommonTestConfig):
    operator_config: Path
    web_host: str
    web_port: int
    plugin_package: str
    plugin_instance: str

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "SemanticActionBoundariesConfig":
        common = CommonTestConfig.from_environment(
            inputs,
            "SEMANTIC_ACTION_BOUNDARIES",
        )
        configured_operator = os.environ.get(
            "SEMANTIC_ACTION_BOUNDARIES_E2E_OPERATOR_CONFIG"
        )
        return cls(
            **common.as_kwargs(),
            operator_config=(
                Path(configured_operator)
                if configured_operator
                else inputs.work_dir / "actraild.conf"
            ),
            web_host=os.environ.get(
                "SEMANTIC_ACTION_BOUNDARIES_E2E_WEB_HOST",
                "127.0.0.1",
            ),
            web_port=resolve_test_port(
                "SEMANTIC_ACTION_BOUNDARIES_E2E_WEB_PORT",
                attempts=common.drain_attempts,
                connect_timeout_seconds=common.drain_interval_seconds,
            ),
            plugin_package="otel-jsonl",
            plugin_instance="v2.semantic-action-boundaries",
        )
