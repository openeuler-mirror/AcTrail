from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.config import CommonTestConfig, TestCaseInputs


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
            # Port 0 requests a kernel-assigned ephemeral port.
            web_port=int(os.environ.get("OTEL_HTTP_E2E_WEB_PORT", "0")),
            plugin_package="otel-http",
            plugin_instance="v2.otel-http",
        )
