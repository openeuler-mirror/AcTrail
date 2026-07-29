from __future__ import annotations

import copy
import json
from pathlib import Path
from typing import Any

from tests.v2.common.output import TestOutput
from tests.v2.common.plugin_test_environment import (
    PluginRuntimeSpec,
    PluginTestEnvironment,
)

from .config import SemanticActionBoundariesConfig


class SemanticActionBoundariesEnvironment(PluginTestEnvironment):
    def __init__(
        self,
        config: SemanticActionBoundariesConfig,
        output: TestOutput,
    ):
        self._operator_config_patch = config.work_dir / "actraild.patch.toml"
        super().__init__(
            config,
            output,
            operator_config=config.operator_config,
            operator_config_patch=self._operator_config_patch,
            web_host=config.web_host,
            web_port=config.web_port,
            plugin=PluginRuntimeSpec(
                package=config.plugin_package,
                instance_id=config.plugin_instance,
                plugin_id="otel-jsonl",
                runtime="builtin",
            ),
        )

    @property
    def export_path(self) -> Path:
        return self.config.work_dir / "semantic-actions.jsonl"

    def prepare(self) -> None:
        self._write_operator_config_patch()
        super().prepare()

    def enable_observed_kinds(
        self,
        observed_kinds: set[str],
    ) -> dict[str, Any]:
        candidate = self.current_config()
        action_kinds = candidate.get("action_kinds")
        if not isinstance(action_kinds, dict):
            raise AssertionError(
                "OTEL observation config has no action_kinds object"
            )
        missing = observed_kinds.difference(action_kinds)
        if missing:
            raise AssertionError(
                "OTEL observation config is missing action kind(s): "
                + ", ".join(sorted(missing))
            )
        for key in action_kinds:
            action_kinds[key] = False
        action_kinds["default"] = False
        for key in observed_kinds:
            action_kinds[key] = True
        candidate["path"] = str(self.export_path)
        candidate["overwrite_enabled"] = True

        returned = self.update_config(copy.deepcopy(candidate))
        if returned.get("action_kinds") != action_kinds:
            raise AssertionError(
                "OTEL observation config returned action_kinds="
                f"{returned.get('action_kinds')!r}, expected {action_kinds!r}"
            )
        if returned.get("path") != str(self.export_path):
            raise AssertionError(
                "OTEL observation path escaped case directory: "
                f"{returned.get('path')!r}"
            )
        return returned

    def _write_operator_config_patch(self) -> None:
        plugin_root = (
            self.config.repo / "examples" / "plugins" / "builtin"
        ).resolve()
        manifest = plugin_root / "otel-jsonl" / "otel-jsonl.plugin.toml"
        if not manifest.is_file():
            raise RuntimeError(
                f"official otel-jsonl manifest not found: {manifest}"
            )
        self._operator_config_patch.write_text(
            "[plugins.discovery]\n"
            f"directory = {json.dumps(str(plugin_root))}\n",
            encoding="utf-8",
        )
