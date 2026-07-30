from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.config import CommonTestConfig, TestCaseInputs


@dataclass(frozen=True)
class OtelJsonlActionFilterConfig(CommonTestConfig):
    operator_config: Path
    web_host: str
    web_port: int
    plugin_package: str
    plugin_instance: str

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "OtelJsonlActionFilterConfig":
        common = CommonTestConfig.from_environment(
            inputs,
            "OTEL_JSONL_ACTION_FILTER",
        )
        configured_operator = os.environ.get(
            "OTEL_JSONL_ACTION_FILTER_E2E_OPERATOR_CONFIG"
        )
        return cls(
            **common.as_kwargs(),
            operator_config=(
                Path(configured_operator)
                if configured_operator
                else inputs.work_dir / "actraild.conf"
            ),
            web_host=os.environ.get(
                "OTEL_JSONL_ACTION_FILTER_E2E_WEB_HOST",
                "127.0.0.1",
            ),
            web_port=int(
                os.environ.get(
                    "OTEL_JSONL_ACTION_FILTER_E2E_WEB_PORT",
                    # Port 0 requests a kernel-assigned ephemeral port.
                    "0",
                )
            ),
            plugin_package="otel-jsonl",
            plugin_instance="v2.otel-jsonl-action-filter",
        )
