from __future__ import annotations

import copy
from typing import Any

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core import TestOutput
from tests.v2.common.plugin_test_environment import (
    PluginRuntimeSpec,
    PluginTestEnvironment,
)

from .config import OtelHttpConfig
from .receiver import OtlpHttpReceiver


class OtelHttpEnvironment(PluginTestEnvironment):
    #: Stand-in for the receiving service's per-user API key.
    EXPORT_CREDENTIAL = "v2-otel-http-credential"

    _CONFIGURABLE_ACTION_KINDS = {
        "process.exec",
        "process.exit",
        "agent.identity",
        "agent.exit",
        "file.modify",
        "file.read",
        "file.write",
        "file.bulk_read",
        "fs.enumerate",
        "http.message",
        "llm.call",
        "llm.request",
        "llm.response",
        "mcp.tool_call",
        "mcp.request",
        "mcp.response",
        "mcp.stdin",
        "mcp.stdout",
        "sse.stream",
        "sse.event",
        "enforcement.decision",
        "process.fork_attempt",
        "agent.invocation",
        "command.invocation",
    }

    def __init__(self, config: OtelHttpConfig, output: TestOutput):
        self._operator_config_patch = config.work_dir / "actraild.patch.toml"
        self._receiver = OtlpHttpReceiver()
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
                plugin_id="otel-http",
                runtime="builtin",
            ),
        )

    def prepare(self) -> None:
        self._write_operator_config_patch()
        self._receiver.start()
        document = super().prepare()
        self._require_safe_schema(document)

    def cleanup(self):
        try:
            return super().cleanup()
        finally:
            self._receiver.stop()

    def configure_buffered_export(self) -> dict[str, Any]:
        candidate = self.current_config()
        action_kinds = candidate.get("action_kinds")
        if not isinstance(action_kinds, dict):
            raise AssertionError("OTEL/HTTP config has no action_kinds object")
        required = {"process.exec", "process.exit"}
        missing = required.difference(action_kinds)
        if missing:
            raise AssertionError(
                "OTEL/HTTP config is missing action kind(s): "
                + ", ".join(sorted(missing))
            )
        for key in action_kinds:
            action_kinds[key] = False
        action_kinds["default"] = False
        for key in required:
            action_kinds[key] = True
        candidate.update(
            {
                "endpoint": self._receiver.endpoint,
                "allow_insecure": True,
                "encoding": "json",
                "compression": "none",
                "attribute_mode": "metadata-only",
                "queue_capacity": 128,
                # Keep the tail buffered until lifecycle replacement calls finish.
                "batch_max_spans": 4096,
                "batch_timeout_ms": 60000,
                "connect_timeout_ms": 250,
                "request_timeout_ms": 500,
                "retry_max_attempts": 1,
                "retry_backoff_ms": 1,
                "shutdown_flush_deadline_ms": 3000,
                # Ingest endpoints that attribute a trace to an account read the
                # sender identity from a request header, never from span content.
                "headers": [
                    {
                        "name": OtlpHttpReceiver.CREDENTIAL_HEADER,
                        "value": self.EXPORT_CREDENTIAL,
                    }
                ],
            }
        )
        returned = self.update_config(copy.deepcopy(candidate))
        expected = {
            "endpoint": self._receiver.endpoint,
            "allow_insecure": True,
            "encoding": "json",
            "compression": "none",
            "attribute_mode": "metadata-only",
            "action_kinds": action_kinds,
            "headers": [
                {
                    "name": OtlpHttpReceiver.CREDENTIAL_HEADER,
                    "value": self.EXPORT_CREDENTIAL,
                }
            ],
        }
        for key, value in expected.items():
            if returned.get(key) != value:
                raise AssertionError(
                    f"OTEL/HTTP returned {key}={returned.get(key)!r}, "
                    f"expected {value!r}"
                )
        return returned

    def finish_buffered_export(self) -> None:
        """Replace the live config, which finishes the previous consumer."""

        candidate = self.current_config()
        if candidate.get("batch_timeout_ms") != 60000:
            raise AssertionError("OTEL/HTTP buffered config is not active")
        candidate["batch_timeout_ms"] = 59000
        returned = self.update_config(candidate)
        if returned.get("batch_timeout_ms") != 59000:
            raise AssertionError("OTEL/HTTP lifecycle replacement was not applied")

    def documents(self) -> list[dict[str, Any]]:
        return self._receiver.documents()

    def credentials(self) -> list[str | None]:
        return self._receiver.credentials()

    def _write_operator_config_patch(self) -> None:
        plugin_root = (
            self.config.repo / "examples" / "plugins" / "builtin"
        ).resolve()
        manifest = plugin_root / "otel-http" / "otel-http.plugin.toml"
        if not manifest.is_file():
            raise RuntimeError(f"official otel-http manifest not found: {manifest}")
        ActrailRuntime.write_isolated_operator_config_patch(
            self._operator_config_patch,
            self.config.work_dir,
            plugin_directory=plugin_root,
        )

    @staticmethod
    def _require_safe_schema(document: dict[str, Any]) -> None:
        if document.get("available") is not True or document.get("editable") is not True:
            raise AssertionError(f"OTEL/HTTP config is not editable: {document}")
        try:
            schema = document["schema"]
            properties = schema["properties"]
            attribute_mode = properties["attribute_mode"]
            action_kinds = properties["action_kinds"]
        except (KeyError, TypeError) as error:
            raise AssertionError("OTEL/HTTP schema has no outbound policy") from error
        if attribute_mode.get("enum") != ["metadata-only", "full"]:
            raise AssertionError("OTEL/HTTP schema has invalid attribute_mode choices")
        if attribute_mode.get("default") != "metadata-only":
            raise AssertionError("OTEL/HTTP schema has an unsafe attribute_mode default")
        if action_kinds.get("additionalProperties") is not False:
            raise AssertionError("OTEL/HTTP schema allows unknown action kinds")
        schema_kinds = action_kinds.get("properties")
        if not isinstance(schema_kinds, dict):
            raise AssertionError("OTEL/HTTP schema has no action kind properties")
        missing_schema_kinds = OtelHttpEnvironment._CONFIGURABLE_ACTION_KINDS.difference(
            schema_kinds
        )
        if missing_schema_kinds:
            raise AssertionError(
                "OTEL/HTTP schema is missing action kind(s): "
                + ", ".join(sorted(missing_schema_kinds))
            )
        if "file.tty_io" in schema_kinds:
            raise AssertionError("OTEL/HTTP schema exposes filtered file.tty_io")
        if schema_kinds.get("default", {}).get("const") is not False:
            raise AssertionError("OTEL/HTTP schema allows blanket action enablement")
        config = document.get("config")
        if not isinstance(config, dict):
            raise AssertionError("OTEL/HTTP config document has no config object")
        if config.get("attribute_mode") != "metadata-only":
            raise AssertionError("official OTEL/HTTP config is not metadata-only")
        configured_kinds = config.get("action_kinds")
        if not isinstance(configured_kinds, dict) or configured_kinds.get("default") is not False:
            raise AssertionError("official OTEL/HTTP action selection is not explicit")
        missing_config_kinds = OtelHttpEnvironment._CONFIGURABLE_ACTION_KINDS.difference(
            configured_kinds
        )
        if missing_config_kinds:
            raise AssertionError(
                "official OTEL/HTTP config is missing action kind(s): "
                + ", ".join(sorted(missing_config_kinds))
            )
