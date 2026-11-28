"""Own alert plugin instances and the isolated daemon scope."""
from __future__ import annotations

from contextlib import ExitStack
import json
import os
from pathlib import Path
from typing import Any

from tests.e2e.common.actrail_runtime import ActrailRuntime
from tests.e2e.common.core import TestOutput
from tests.e2e.common.tool_alert_lifecycle.plugin_package import ToolAlertPluginPackage

from tests.e2e.common.plugin_instance import PluginInstance, PluginInstanceSpec

from .config import ToolConsecutiveFailureAlertConfig

from .constants import CASE_CONSTANTS

class ToolConsecutiveFailureAlertEnvironment:
    def __init__(
        self,
        config: ToolConsecutiveFailureAlertConfig,
        output: TestOutput,
    ):
        self.config = config
        self.output = output
        self._repo_package = ToolAlertPluginPackage(
            config.repo, config.work_dir, config.plugin_manifest
        )
        self.runtime = ActrailRuntime.isolated(
            config.repo,
            config.bin_dir,
            config.command_timeout_seconds,
            output,
            config.work_dir,
        )
        self._repo_plugin = PluginInstance(PluginInstanceSpec(
            self._repo_package.manifest, config.plugin_instance, ("alert-write",)
        ))
        self._installed_plugin = PluginInstance(PluginInstanceSpec(
            self.installed_manifest, config.installed_plugin_instance, ("alert-write",)
        ))

    @property
    def database(self) -> Path:
        return self.config.work_dir / "data" / "actrail.sqlite"

    @property
    def plugin_root(self) -> Path:
        configured = os.environ.get("ACTRAIL_PLUGIN_DIR")
        if configured:
            return Path(configured)
        return Path.home() / ".actrail" / "plugins"

    @property
    def installed_package_dir(self) -> Path:
        return self.plugin_root / "tool-consecutive-failure-alert"

    @property
    def installed_manifest(self) -> Path:
        return (
            self.installed_package_dir
            / "tool-consecutive-failure-alert.plugin.toml"
        )

    def prepare(self) -> None:
        self._repo_package.prepare()
        self.runtime.prepare()
        self.load_repo_plugin()

    def load_repo_plugin(self, plugin_config: Path | None = None) -> None:
        self._repo_plugin.load(plugin_config)

    def unload_repo_plugin(self) -> None:
        self._repo_plugin.unload()

    def plugin_load_command(self, *, plugin_config: Path | None = None) -> list[Path | str]:
        return self._repo_plugin.load_command(plugin_config)

    def plugin_unload_command(self) -> list[Path | str]:
        return self._repo_plugin.unload_command()

    def plugin_status_command(self) -> list[Path | str]:
        return self._repo_plugin.status_command()

    def launch_command(
        self,
        marker: str,
        command: list[str],
    ) -> list[Path | str]:
        return [
            self.runtime.actrailctl,
            "--config",
            self.config.operator_config,
            "launch",
            "--name",
            marker,
            "--",
            *command,
        ]

    def load_installed_plugin(self) -> None:
        self._installed_plugin.load()

    def unload_installed_plugin(self) -> None:
        self._installed_plugin.unload()

    def plugin_status(self) -> dict[str, str]:
        return self._repo_plugin.status()

    def viewer_actions(self, trace_id: int) -> list[dict[str, Any]]:
        result = self.runtime.run(
            [
                self.runtime.actrailviewer,
                "--config",
                self.config.operator_config,
                "--output-format",
                "json",
                "actions",
                "--trace-id",
                str(trace_id),
            ],
            echo=False,
        )
        if result.returncode != 0:
            raise AssertionError(
                "actrailviewer actions exited with "
                f"{result.returncode}: {result.output[-CASE_CONSTANTS.command_diagnostic_tail_characters:]}"
            )
        try:
            document = json.loads(result.stdout)
        except json.JSONDecodeError as error:
            raise AssertionError(
                "actrailviewer actions returned invalid JSON"
            ) from error
        if not isinstance(document, dict):
            raise AssertionError(
                "actrailviewer actions returned non-object JSON"
            )
        actions = document.get("actions")
        if not isinstance(actions, list):
            raise AssertionError(
                "actrailviewer actions returned no actions array"
            )
        return actions

    def __enter__(self):
        self._scope = ExitStack()
        self._scope.enter_context(self.runtime)
        # 注册卸载顺序后再准备环境，部分启动失败也会释放资源。
        self._scope.push(self._repo_plugin)
        self._scope.push(self._installed_plugin)
        return self

    def __exit__(self, exception_type, exception, traceback):
        return self._scope.__exit__(exception_type, exception, traceback)
