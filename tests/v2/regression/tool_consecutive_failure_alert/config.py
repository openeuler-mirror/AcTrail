from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.config import CommonTestConfig, TestCaseInputs


@dataclass(frozen=True)
class ToolConsecutiveFailureAlertConfig(CommonTestConfig):
    operator_config: Path
    plugin_manifest: Path
    plugin_instance: str
    installed_plugin_instance: str

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "ToolConsecutiveFailureAlertConfig":
        common = CommonTestConfig.from_environment(
            inputs,
            "TOOL_CONSECUTIVE_FAILURE_ALERT",
        )
        configured_operator = os.environ.get(
            "TOOL_CONSECUTIVE_FAILURE_ALERT_E2E_OPERATOR_CONFIG"
        )
        return cls(
            **common.as_kwargs(),
            operator_config=(
                Path(configured_operator)
                if configured_operator
                else inputs.work_dir / "actraild.conf"
            ),
            plugin_manifest=(
                inputs.repo
                / "examples"
                / "plugins"
                / "wasm-legacy"
                / "tool-consecutive-failure-alert"
                / "plugin.toml"
            ),
            plugin_instance="tool-alert.v2",
            installed_plugin_instance="tool-alert.installed",
        )
