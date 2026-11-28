from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.e2e.common.core import CommonTestConfig, TestRunDirectories

from .constants import CASE_CONSTANTS


@dataclass(frozen=True)
class ToolConsecutiveFailureAlertConfig(CommonTestConfig):
    operator_config: Path
    plugin_manifest: Path
    plugin_instance: str
    installed_plugin_instance: str

    @classmethod
    def from_environment(
        cls,
        directories: TestRunDirectories,
    ) -> "ToolConsecutiveFailureAlertConfig":
        common = CommonTestConfig.from_environment(
            directories,
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
                else directories.work_dir / "actraild.conf"
            ),
            plugin_manifest=(
                directories.repo
                / "examples"
                / "plugins"
                / "wasm-legacy"
                / "tool-consecutive-failure-alert"
                / "plugin.toml"
            ),
            plugin_instance=CASE_CONSTANTS.plugin_instance,
            installed_plugin_instance=CASE_CONSTANTS.installed_plugin_instance,
        )
