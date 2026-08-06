from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.output import TestOutput
from tests.v2.common.test_case import TestResult, TestStatus

from .config import ToolConsecutiveFailureAlertConfig


class ToolConsecutiveFailureAlertEnvironment:
    def __init__(
        self,
        config: ToolConsecutiveFailureAlertConfig,
        output: TestOutput,
    ):
        self.config = config
        self.output = output
        self.runtime = ActrailRuntime.isolated(
            config.repo,
            config.bin_dir,
            config.command_timeout_seconds,
            output,
            config.work_dir,
        )
        self._plugin_loaded = False
        self._installed_plugin_loaded = False

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
        self.runtime.prepare()
        self.load_repo_plugin()

    def load_repo_plugin(self) -> None:
        if self._plugin_loaded:
            return
        load = self.runtime.run_checked(self.plugin_load_command())
        if "loaded instance=" not in load.output:
            raise AssertionError(
                "plugin load did not report a loaded instance: "
                f"{load.output[-4000:]}"
            )
        self._plugin_loaded = True

    def unload_repo_plugin(self) -> None:
        if not self._plugin_loaded:
            return
        result = self.runtime.run(
            self.plugin_unload_command(),
            echo=False,
        )
        if result.returncode != 0:
            raise AssertionError(
                f"plugin unload exited with {result.returncode}: "
                f"{result.output[-1000:]}"
            )
        self._plugin_loaded = False

    def plugin_load_command(self) -> list[Path | str]:
        return self._plugin_load_command(
            self.config.plugin_manifest,
            self.config.plugin_instance,
        )

    def _plugin_load_command(
        self,
        manifest: Path,
        instance: str,
    ) -> list[Path | str]:
        return [
            self.runtime.actraild,
            "--config",
            self.config.operator_config,
            "plugin",
            "load",
            "--manifest",
            manifest,
            "--instance",
            instance,
            "--grant",
            "alert-write",
        ]

    def plugin_unload_command(self) -> list[Path | str]:
        return self._plugin_unload_command(self.config.plugin_instance)

    def _plugin_unload_command(self, instance: str) -> list[Path | str]:
        return [
            self.runtime.actraild,
            "--config",
            self.config.operator_config,
            "plugin",
            "unload",
            "--instance",
            instance,
        ]

    def plugin_status_command(self) -> list[Path | str]:
        return [
            self.runtime.actraild,
            "--config",
            self.config.operator_config,
            "plugin",
            "status",
            "--instance",
            self.config.plugin_instance,
        ]

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
        if self._installed_plugin_loaded:
            return
        load = self.runtime.run_checked(
            self._plugin_load_command(
                self.installed_manifest,
                self.config.installed_plugin_instance,
            )
        )
        if "loaded instance=" not in load.output:
            raise AssertionError(
                "installed package load did not report a loaded instance: "
                f"{load.output[-4000:]}"
            )
        self._installed_plugin_loaded = True

    def unload_installed_plugin(self) -> None:
        if not self._installed_plugin_loaded:
            return
        result = self.runtime.run(
            self._plugin_unload_command(
                self.config.installed_plugin_instance,
            ),
            echo=False,
        )
        if result.returncode != 0:
            raise AssertionError(
                f"installed package unload exited with {result.returncode}: "
                f"{result.output[-1000:]}"
            )
        self._installed_plugin_loaded = False

    def plugin_status(self) -> dict[str, str]:
        result = self.runtime.run(self.plugin_status_command(), echo=False)
        if result.returncode != 0:
            raise AssertionError(
                f"plugin status exited with {result.returncode}: "
                f"{result.output[-2000:]}"
            )
        fields: dict[str, str] = {}
        for token in result.stdout.split():
            if "=" in token:
                key, value = token.split("=", 1)
                fields[key] = value
        return fields

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
                f"{result.returncode}: {result.output[-2000:]}"
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

    def cleanup(self) -> TestResult | None:
        failures: list[str] = []
        if self._installed_plugin_loaded:
            try:
                self.unload_installed_plugin()
            except Exception as error:
                failures.append(f"installed package unload exception: {error}")
        if self._plugin_loaded:
            try:
                unload = self.runtime.run(
                    self.plugin_unload_command(),
                    echo=False,
                )
                if unload.returncode != 0:
                    failures.append(f"plugin unload: {unload.output[-1000:]}")
            except Exception as error:
                failures.append(f"plugin unload exception: {error}")
        stop = self.runtime.stop()
        if stop is not None and stop.returncode != 0:
            failures.append(f"daemon stop: {stop.output[-1000:]}")
        if failures:
            return TestResult(
                TestStatus.FAILED,
                "; ".join(failures),
            )
        return TestResult(
            TestStatus.PASSED,
            "plugin unloaded and daemon stopped",
        )
