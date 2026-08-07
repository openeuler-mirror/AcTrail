from __future__ import annotations

import copy
import json
from pathlib import Path
from typing import Any

from tests.v2.common.core import TestOutput
from tests.v2.common.plugin_test_environment import (
    PluginRuntimeSpec,
    PluginTestEnvironment,
)

from .config import OtelJsonlActionFilterConfig
from .receiver import JsonRpcOtelReceiver


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
        self._json_rpc_receiver = JsonRpcOtelReceiver("otel.export")
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
        self._json_rpc_receiver.start()
        document = super().prepare()
        self._require_exporter_schema(document)
        self._require_checkbox_schema(document)

    def cleanup(self):
        try:
            return super().cleanup()
        finally:
            self._json_rpc_receiver.stop()

    def update_selection(
        self,
        exporter: str,
        enabled_kinds: set[str],
    ) -> dict[str, Any]:
        if exporter not in {"file", "json_rpc_http"}:
            raise ValueError(f"unknown exporter {exporter}")
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
        candidate["exporter"] = exporter
        if exporter == "file":
            file_config = candidate.get("file")
            if not isinstance(file_config, dict):
                raise AssertionError("plugin config has no file exporter object")
            file_config["path"] = str(self.export_path)
            file_config["overwrite_enabled"] = True
        else:
            json_rpc_config = candidate.get("json_rpc_http")
            if not isinstance(json_rpc_config, dict):
                raise AssertionError(
                    "plugin config has no JSON-RPC HTTP exporter object"
                )
            json_rpc_config.update(
                {
                    "endpoint": self._json_rpc_receiver.endpoint,
                    "method": "otel.export",
                    "connect_timeout_ms": 250,
                    "request_timeout_ms": 500,
                    "response_body_max_bytes": 65536,
                    "max_attempts": 3,
                    "retry_backoff_ms": 10,
                }
            )

        returned = self.update_config(copy.deepcopy(candidate))
        if returned.get("action_kinds") != action_kinds:
            raise AssertionError(
                f"plugin returned action_kinds={returned.get('action_kinds')!r}, "
                f"expected {action_kinds!r}"
            )
        if returned.get("exporter") != exporter:
            raise AssertionError(
                f"plugin returned exporter={returned.get('exporter')!r}, "
                f"expected {exporter!r}"
            )
        if exporter == "file":
            returned_file = returned.get("file")
            if (
                not isinstance(returned_file, dict)
                or returned_file.get("path") != str(self.export_path)
            ):
                raise AssertionError(
                    "plugin export path escaped case directory: "
                    f"{returned_file!r}"
                )
        else:
            returned_json_rpc = returned.get("json_rpc_http")
            if (
                not isinstance(returned_json_rpc, dict)
                or returned_json_rpc.get("endpoint")
                != self._json_rpc_receiver.endpoint
            ):
                raise AssertionError(
                    "plugin returned unexpected JSON-RPC endpoint: "
                    f"{returned_json_rpc!r}"
                )
        return returned

    def fail_next_json_rpc_requests(self, count: int) -> None:
        self._json_rpc_receiver.fail_next_requests(count)

    def delay_next_json_rpc_responses(
        self,
        delay_seconds: float,
        count: int = 1,
    ) -> None:
        self._json_rpc_receiver.delay_next_responses(
            delay_seconds,
            count,
        )

    @property
    def json_rpc_injected_failures(self) -> int:
        return self._json_rpc_receiver.injected_failures

    @property
    def json_rpc_injected_response_delays(self) -> int:
        return self._json_rpc_receiver.injected_response_delays

    def json_rpc_request_ids(self) -> list[int]:
        return self._json_rpc_receiver.request_ids()

    def json_rpc_documents(self) -> list[dict[str, Any]]:
        return self._json_rpc_receiver.documents()

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
    def _require_exporter_schema(cls, document: dict[str, Any]) -> None:
        schema = document.get("schema")
        try:
            properties = schema["properties"]
            exporter_options = properties["exporter"]["oneOf"]
            file_schema = properties["file"]
            json_rpc_schema = properties["json_rpc_http"]
        except (KeyError, TypeError) as error:
            raise AssertionError(
                "schema has no selectable exporter branches"
            ) from error
        choices = {
            option.get("const")
            for option in exporter_options
            if isinstance(option, dict)
        }
        if choices != {"file", "json_rpc_http"}:
            raise AssertionError(
                f"schema exporter choices are invalid: {choices}"
            )
        if "path" not in file_schema.get("properties", {}):
            raise AssertionError("schema file exporter has no path")
        json_rpc_properties = json_rpc_schema.get("properties", {})
        if not {"endpoint", "method"}.issubset(json_rpc_properties):
            raise AssertionError(
                "schema JSON-RPC exporter has no endpoint or method"
            )
        if schema.get("then", {}).get("required") != ["file"]:
            raise AssertionError("schema does not select the file branch")
        if schema.get("else", {}).get("required") != ["json_rpc_http"]:
            raise AssertionError("schema does not select the JSON-RPC branch")

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
