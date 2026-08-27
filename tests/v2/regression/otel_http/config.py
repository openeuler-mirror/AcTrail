from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import CommonTestConfig, TestCaseInputs
from tests.v2.common.core.loopback_port import resolve_test_port


@dataclass(frozen=True)
class OtelHttpConfig(CommonTestConfig):
    operator_config: Path
    web_host: str
    web_port: int
    plugin_package: str
    plugin_instance: str

    @classmethod
    def from_environment(cls, inputs: TestCaseInputs) -> "OtelHttpConfig":
        common = CommonTestConfig.from_environment(inputs, "OTEL_HTTP")
        configured_operator = os.environ.get("OTEL_HTTP_E2E_OPERATOR_CONFIG")
        return cls(
            **common.as_kwargs(),
            operator_config=(
                Path(configured_operator)
                if configured_operator
                else inputs.work_dir / "actraild.conf"
            ),
            web_host=os.environ.get("OTEL_HTTP_E2E_WEB_HOST", "127.0.0.1"),
            web_port=resolve_test_port(
                "OTEL_HTTP_E2E_WEB_PORT",
                attempts=common.drain_attempts,
                connect_timeout_seconds=common.drain_interval_seconds,
            ),
            plugin_package="otel-http",
            plugin_instance="v2.otel-http",
        )
