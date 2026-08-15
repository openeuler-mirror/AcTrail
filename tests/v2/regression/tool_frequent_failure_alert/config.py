from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import CommonTestConfig, TestCaseInputs


@dataclass(frozen=True)
class ToolFrequentFailureAlertConfig(CommonTestConfig):
    operator_config: Path
    plugin_manifest: Path
    plugin_config: Path
    agent_plugin_config: Path
    plugin_instance: str
    installed_plugin_instance: str

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "ToolFrequentFailureAlertConfig":
        common = CommonTestConfig.from_environment(
            inputs,
            "TOOL_FREQUENT_FAILURE_ALERT",
        )
        configured_operator = os.environ.get(
            "TOOL_FREQUENT_FAILURE_ALERT_E2E_OPERATOR_CONFIG"
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
                / "tool-frequent-failure-alert"
                / "plugin.toml"
            ),
            plugin_config=(
                Path(__file__).parent / "tool-frequent-failure-alert.e2e.config.json"
            ),
            agent_plugin_config=(
                Path(__file__).parent
                / "tool-frequent-failure-alert.agent.e2e.config.json"
            ),
            plugin_instance="tool-frequent-alert.v2",
            installed_plugin_instance="tool-frequent-alert.installed",
        )
