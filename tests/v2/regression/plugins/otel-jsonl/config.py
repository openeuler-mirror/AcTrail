from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.config import CommonTestConfig, TestCaseInputs


@dataclass(frozen=True)
class OtelJsonlConfig(CommonTestConfig):
    operator_config: Path
    web_host: str
    web_port: int
    plugin_package: str
    plugin_instance: str

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "OtelJsonlConfig":
        common = CommonTestConfig.from_environment(inputs, "OTEL_JSONL")
        return cls(
            **common.as_kwargs(),
            operator_config=Path(
                os.environ.get(
                    "OTEL_JSONL_E2E_OPERATOR_CONFIG",
                    "/etc/actrail/actraild.conf",
                )
            ),
            web_host=os.environ.get("OTEL_JSONL_E2E_WEB_HOST", "127.0.0.1"),
            web_port=int(os.environ.get("OTEL_JSONL_E2E_WEB_PORT", "18080")),
            plugin_package="otel-jsonl",
            plugin_instance="v2.otel-jsonl",
        )
