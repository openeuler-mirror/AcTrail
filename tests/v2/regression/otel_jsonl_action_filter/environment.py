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

from .config import OtelJsonlActionFilterConfig


class OtelJsonlActionFilterEnvironment(PluginTestEnvironment):
    _REPRESENTATIVE_KINDS = {
        "process.exec",
        "process.exit",
        "agent.identity",
        "agent.exit",
        "file.read",
        "command.invocation",
        "llm.call",
        "llm.request",
        "llm.response",
    }

    def __init__(
        self,
        config: OtelJsonlActionFilterConfig,
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
        return self.config.work_dir / "otel-action-filter.jsonl"

    def prepare(self) -> None:
        self._write_operator_config_patch()
        document = super().prepare()
        self._require_checkbox_schema(document)

    def update_selection(self, enabled_kinds: set[str]) -> dict[str, Any]:
        candidate = self.current_config()
        action_kinds = candidate.get("action_kinds")
        if not isinstance(action_kinds, dict):
            raise AssertionError("plugin config has no action_kinds object")
        missing = enabled_kinds.difference(action_kinds)
        if missing:
            raise AssertionError(
                "plugin config is missing action kind(s): "
                + ", ".join(sorted(missing))
            )
        for key in action_kinds:
            action_kinds[key] = False
        action_kinds["default"] = False
        for key in enabled_kinds:
            action_kinds[key] = True
        candidate["path"] = str(self.export_path)
        candidate["overwrite_enabled"] = True

        returned = self.update_config(copy.deepcopy(candidate))
        if returned.get("action_kinds") != action_kinds:
            raise AssertionError(
                f"plugin returned action_kinds={returned.get('action_kinds')!r}, "
                f"expected {action_kinds!r}"
            )
        if returned.get("path") != str(self.export_path):
            raise AssertionError(
                f"plugin export path escaped case directory: "
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

    @classmethod
    def _require_checkbox_schema(cls, document: dict[str, Any]) -> None:
        if (
            document.get("available") is not True
            or document.get("editable") is not True
        ):
            raise AssertionError(f"plugin config is not editable: {document}")
        schema = document.get("schema")
        try:
            action_kinds_schema = schema["properties"]["action_kinds"]
            properties = action_kinds_schema["properties"]
        except (KeyError, TypeError) as error:
            raise AssertionError(
                "schema has no action_kinds properties"
            ) from error
        if action_kinds_schema.get("additionalProperties") is not False:
            raise AssertionError("schema allows unknown action kinds")
        if "file.tty_io" in properties:
            raise AssertionError("schema exposes file.tty_io")
        required = {"default", *cls._REPRESENTATIVE_KINDS}
        invalid = sorted(
            key
            for key in required
            if not isinstance(properties.get(key), dict)
            or properties[key].get("type") != "boolean"
        )
        if invalid:
            raise AssertionError(
                "schema action kind(s) are not boolean: "
                + ", ".join(invalid)
            )
        config = document.get("config")
        action_kinds = (
            config.get("action_kinds")
            if isinstance(config, dict)
            else None
        )
        if not isinstance(action_kinds, dict):
            raise AssertionError("plugin config has no action_kinds object")
        if "file.tty_io" in action_kinds:
            raise AssertionError("plugin config exposes file.tty_io")
