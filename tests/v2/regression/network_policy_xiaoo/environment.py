from __future__ import annotations

import errno
import json
import os
import re
import shutil
import sqlite3
import time
from contextlib import closing
from dataclasses import replace
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

from tests.v2.common.actrail_runtime import CommandResult
from tests.v2.common.patch_builder import PatchBuilder
from tests.v2.common.core import TestOutput, TestResult, TestStatus
from tests.v2.common.plugin_test_environment import (
    PluginRuntimeSpec,
    PluginTestEnvironment,
)
from tests.v2.regression.command_policy_xiaoo.provider import LocalXiaooProvider

from .config import NetworkPolicyXiaooConfig

from .constants import CASE_CONSTANTS


INSTANCE = "wasm.network-policy-dynamic"
RULE_ID = "xiaoo-provider-deny"
TRACE_RE = re.compile(r"trace trace-(\d+) entered Active")


class NetworkPolicyXiaooEnvironment(PluginTestEnvironment):
    def __init__(
        self,
        config: NetworkPolicyXiaooConfig,
        output: TestOutput,
    ):
        self.marker = config.work_dir / "xiaoo-network.marker"
        self._plugin_root = config.work_dir / "plugins"
        self._rules = config.work_dir / "network-control.rules"
        self._xiaoo_config = config.work_dir / "xiaoo-config.toml"
        self._provider = LocalXiaooProvider(
            config.repo,
            config.work_dir,
            self.marker,
            config.ready_timeout_seconds,
        )
        self._provider_endpoint: str | None = None
        super().__init__(
            config,
            output,
            operator_config=config.operator_config,
            operator_config_patches=[config.operator_config_patch, Path(__file__).with_name("patch_config.toml")],
            web_host=config.web_host,
            web_port=config.web_port,
            plugin=PluginRuntimeSpec(
                package="network-policy-dynamic",
                instance_id=INSTANCE,
                plugin_id=INSTANCE,
                runtime="wasm",
            ),
        )

    @property
    def network_config(self) -> NetworkPolicyXiaooConfig:
        return self.config

    @property
    def provider_endpoint(self) -> str:
        if self._provider_endpoint is None:
            raise RuntimeError("local provider endpoint has not been initialized")
        return self._provider_endpoint

    def prepare(self) -> dict[str, Any]:
        self._install_plugin_package()
        self._rules.write_text("", encoding="utf-8")
        self._write_operator_patch()
        provider_url = self._provider.start()
        self._provider_endpoint = self._endpoint(provider_url)
        if not self._provider_endpoint.startswith("127.0.0.1:"):
            raise RuntimeError(
                f"local provider is not on IPv4 loopback: {self._provider_endpoint}"
            )
        self.plugin = replace(
            self.plugin,
            load_grants={
                "network_policy_rules_apply": [
                    {
                        "decision": "deny",
                        "endpoint_scope": self.provider_endpoint,
                    }
                ]
            },
        )
        self._write_xiaoo_config(provider_url)
        initial = super().prepare()
        self._require_load_grant()
        return initial

    def require_default_route(
        self,
        previous_source_revision: int | None = None,
    ) -> int:
        expected_config = {"rules": []}
        current = self.current_config()
        if current != expected_config:
            raise AssertionError(f"network publisher config is not empty: {current}")
        output, fields = self._dry_run()
        expected = {
            "matched": "false",
            "decision": "allow",
            "rule_id": "none",
            "owner": "none",
            "endpoint": self.provider_endpoint,
            "rule_revision": "none",
        }
        if any(fields.get(key) != value for key, value in expected.items()):
            raise AssertionError(f"provider route was not default-allow: {output}")
        source_revision = self._require_revision(fields, "source_revision", 0)
        if (
            previous_source_revision is not None
            and source_revision <= previous_source_revision
        ):
            raise AssertionError(
                "clearing the network rule did not advance source revision: "
                f"{previous_source_revision} -> {source_revision}"
            )
        return source_revision

    def publish_deny(self, previous_source_revision: int) -> tuple[str, int, int]:
        candidate = {
            "rules": [
                {
                    "rule_id": RULE_ID,
                    "decision": "deny",
                    "endpoint": self.provider_endpoint,
                }
            ]
        }
        updated = self.update_config(candidate)
        if updated != candidate:
            raise AssertionError(f"network policy update returned bad config: {updated}")
        output, fields = self._dry_run()
        expected = {
            "matched": "true",
            "decision": "deny",
            "rule_id": RULE_ID,
            "owner": INSTANCE,
            "endpoint": self.provider_endpoint,
        }
        if any(fields.get(key) != value for key, value in expected.items()):
            raise AssertionError(f"published provider route is incomplete: {output}")
        rule_revision = self._require_revision(fields, "rule_revision", 1)
        source_revision = self._require_revision(fields, "source_revision", 1)
        if source_revision <= previous_source_revision:
            raise AssertionError(
                "publishing the network rule did not advance source revision: "
                f"{previous_source_revision} -> {source_revision}"
            )
        return output, rule_revision, source_revision

    def clear_deny(self, previous_source_revision: int) -> int:
        candidate = {"rules": []}
        updated = self.update_config(candidate)
        if updated != candidate:
            raise AssertionError(f"network policy clear returned bad config: {updated}")
        return self.require_default_route(previous_source_revision)

    def run_xiaoo(self, trace_name: str) -> tuple[int, CommandResult]:
        self.marker.unlink(missing_ok=True)
        environment = os.environ.copy()
        for variable in (
            "ALL_PROXY",
            "HTTPS_PROXY",
            "HTTP_PROXY",
            "all_proxy",
            "https_proxy",
            "http_proxy",
        ):
            environment.pop(variable, None)
        environment["NO_PROXY"] = CASE_CONSTANTS.no_proxy_hosts
        environment["no_proxy"] = CASE_CONSTANTS.no_proxy_hosts
        environment["ACTRAIL_XIAOO_NETWORK_POLICY_KEY"] = "local-test-key"
        environment["PATH"] = "/usr/bin"
        result = self.runtime.run(
            self.runtime.control_command(
                "launch",
                "--seccomp-notify",
                "required",
                "--name",
                trace_name,
                "--",
                self.network_config.xiaoo_binary,
                "--cli",
                "run",
                "--config",
                self._xiaoo_config,
                "--tools",
                "bash",
                "--max-turns",
                str(CASE_CONSTANTS.max_turns),
                "--debug",
                "--prompt",
                "Use the Bash tool exactly once, then report its operating-system result.",
            ),
            timeout_seconds=self.network_config.launch_timeout_seconds,
            environment=environment,
        )
        trace_ids = TRACE_RE.findall(result.output)
        if len(trace_ids) != 1:
            raise AssertionError(
                f"Xiaoo output did not contain exactly one trace ID: {result.output}"
            )
        if "enforcement-network-connect-seccomp" not in result.output:
            raise AssertionError("launch omitted network-connect enforcement capability")
        if "seccomp_notify:enabled" not in result.output:
            raise AssertionError("launch did not select seccomp user-notify")
        return int(trace_ids[0]), result

    def require_allowed(self, result: CommandResult) -> None:
        if result.returncode != 0:
            raise AssertionError(
                f"default-allowed Xiaoo exited with {result.returncode}: {result.output}"
            )
        if not self.marker.is_file():
            raise AssertionError("default-allowed Xiaoo did not create its Bash marker")
        marker = self.marker.read_text(encoding="utf-8").strip()
        if marker != "ACTRAIL_XIAOO_COMMAND_OK":
            raise AssertionError(f"default-allowed Xiaoo marker is wrong: {marker!r}")

    def require_denied(self, result: CommandResult) -> None:
        if self.marker.exists():
            raise AssertionError("network-denied Xiaoo reached its Bash tool")
        if result.returncode == 0:
            raise AssertionError("network-denied Xiaoo unexpectedly exited successfully")
        lowered = result.output.lower()
        if "connection failed" not in lowered or "llm provider error" not in lowered:
            raise AssertionError(
                f"Xiaoo did not expose the provider connection denial: {result.output}"
            )

    def wait_for_evidence(
        self,
        trace_id: int,
        rule_revision: int,
    ) -> dict[str, str]:
        deadline = time.monotonic() + self.network_config.evidence_timeout_seconds
        while time.monotonic() < deadline:
            self.runtime.run_checked(self.runtime.control_command("list-traces"))
            governed = []
            for event in self._network_events(trace_id):
                payload = event["payload"]
                if (
                    event.get("collector") == "network-control"
                    and payload.get("endpoint") == self.provider_endpoint
                ):
                    governed.append(payload)
            if governed:
                expected_metadata = {
                    "subject": "network-action",
                    "operation": "connect",
                    "decision": "deny",
                    "decision_source": "rule",
                    "rule_id": RULE_ID,
                    "policy_owner_instance_id": INSTANCE,
                    "policy_endpoint_scope": self.provider_endpoint,
                    "rule_revision": str(rule_revision),
                }
                for payload in governed:
                    metadata = payload.get("metadata")
                    if (
                        payload.get("transport") != "inet"
                        or payload.get("result") != -errno.EPERM
                        or not isinstance(metadata, dict)
                        or any(
                            metadata.get(key) != value
                            for key, value in expected_metadata.items()
                        )
                    ):
                        raise AssertionError(
                            "provider connect has incomplete deny attribution: "
                            f"{payload}"
                        )
                return governed[0]["metadata"]
            time.sleep(self.network_config.drain_interval_seconds)
        raise AssertionError(
            f"trace-{trace_id} has no governed provider-connect evidence"
        )

    def wait_for_boundary_alerts(self, trace_id: int) -> list[dict[str, Any]]:
        deadline = time.monotonic() + self.network_config.evidence_timeout_seconds
        alerts = []
        while time.monotonic() < deadline:
            self.runtime.run_checked(self.runtime.control_command("list-traces"), echo=False)
            alerts = self.api.alerts(trace_id).get("alerts", [])
            with closing(sqlite3.connect(f"file:{self.runtime.database_path}?mode=ro", uri=True)) as connection:
                stored = connection.execute(
                    "SELECT a.alert_id, d.kind, a.payload_json FROM alerts a "
                    "JOIN alert_definitions d USING(alert_definition_id) "
                    "WHERE a.trace_id = ? AND d.producer_plugin_id = ? ORDER BY a.alert_id",
                    (trace_id, "actraild.enforcement"),
                ).fetchall()
                if not alerts or sorted(alert["alert_id"] for alert in alerts) != [row[0] for row in stored]:
                    time.sleep(self.network_config.drain_interval_seconds)
                    continue
                persisted = {identifier: (kind, json.loads(payload)) for identifier, kind, payload in stored}
                for alert in alerts:
                    expected = {
                        "definition_key": "network-connection-boundary-violation",
                        "kind": "network.connection.boundary-violation",
                        "producer_plugin_id": "actraild.enforcement",
                        "severity": "high",
                    }
                    payload = alert.get("payload", {})
                    expected_payload = {
                        "operation": "connect",
                        "endpoint": self.provider_endpoint,
                        "endpoint_side": "remote",
                        "rule_id": RULE_ID,
                        "policy_owner_instance_id": INSTANCE,
                        "decision_source": "fast-path-deny",
                    }
                    if (
                        any(alert.get(key) != value for key, value in expected.items())
                        or any(payload.get(key) != value for key, value in expected_payload.items())
                        or persisted[alert["alert_id"]] != (alert["kind"], payload)
                        or not connection.execute(
                            "SELECT 1 FROM processes WHERE process_id = ?", (payload.get("process_id"),)
                        ).fetchone()
                    ):
                        raise AssertionError(f"network boundary alert lost its policy or process attribution: {alert}")
                return alerts
        raise AssertionError(f"trace-{trace_id} network boundary alerts did not reach SQLite and Web: {alerts}")

    def finish_boundary_assertions(self, allowed_trace_ids: tuple[int, ...], denied_trace_id: int) -> None:
        self.unload_plugin()
        failures: list[str] = []
        self._stop_web(failures)
        if failures:
            raise AssertionError("; ".join(failures))
        stopped = self.runtime.stop()
        if stopped is None or stopped.returncode != 0:
            raise AssertionError(f"daemon failed to drain alerts on shutdown: {stopped}")
        self._daemon_started = False
        try:
            denials = [event for event in self._network_events(denied_trace_id)
                       if event.get("collector") == "network-control"
                       and event["payload"].get("endpoint") == self.provider_endpoint
                       and event["payload"].get("result") == -errno.EPERM]
            with closing(sqlite3.connect(f"file:{self.runtime.database_path}?mode=ro", uri=True)) as connection:
                rows = connection.execute(
                    "SELECT a.payload_json FROM alerts a "
                    "JOIN alert_definitions d USING(alert_definition_id) "
                    "WHERE a.trace_id = ? AND d.producer_plugin_id = ? AND d.kind = ? ORDER BY a.alert_id",
                    (denied_trace_id, "actraild.enforcement", "network.connection.boundary-violation"),
                ).fetchall()
                summaries = [json.loads(row[0])["aggregation"] for row in rows]
                counts = [summary["received_denials"] for summary in summaries]
                expected_counts = [1, len(denials) - 1] if len(denials) > 1 else [1]
                if not denials or counts != expected_counts or any(
                    summary["backend"] != "seccomp-user-notify" for summary in summaries
                ):
                    raise AssertionError(f"network denial aggregation lost counts or bounds: {len(denials)} denials, {summaries}")
                for trace_id in allowed_trace_ids:
                    count = connection.execute(
                        "SELECT COUNT(*) FROM alerts a "
                        "JOIN alert_definitions d USING(alert_definition_id) "
                        "WHERE a.trace_id = ? AND d.kind = ?",
                        (trace_id, "network.connection.boundary-violation"),
                    ).fetchone()[0]
                    if count:
                        raise AssertionError(f"allowed trace-{trace_id} produced {count} network boundary alerts")
        finally:
            cleaned = self.runtime.clean(echo=False)
            if cleaned.returncode != 0:
                raise AssertionError(f"trace cleanup failed: {cleaned.output}")

    def cleanup(self) -> TestResult:
        base = super().cleanup()
        provider_failure = self._provider.stop()
        if provider_failure is None:
            return base
        return TestResult(
            TestStatus.COMPOSITE,
            "network-policy environment cleanup",
            {
                "services": base,
                "provider": TestResult(TestStatus.FAILED, provider_failure),
            },
        )

    def _install_plugin_package(self) -> None:
        source = self.network_config.plugin_package
        destination = self._plugin_root / "network-policy-dynamic"
        destination.mkdir(parents=True)
        assets = {
            "network-policy-dynamic.plugin.toml": "network-policy-dynamic.plugin.toml",
            "network-policy-dynamic.config.json": "network-policy-dynamic.config.json",
            "component-network-policy-dynamic.wasm": "component-network-policy-dynamic.wasm",
            "config.schema.json": "config.schema.json",
        }
        for source_name, destination_name in assets.items():
            source_path = source / source_name
            if not source_path.is_file():
                raise RuntimeError(f"official network-policy asset missing: {source_path}")
            shutil.copy2(source_path, destination / destination_name)

    def _write_operator_patch(self) -> None:
        (
            PatchBuilder()
            .with_work_dir(self.config.work_dir)
            .with_values({"network_control": {
                "rules_path": str(self._rules),
                "alert_window_ms": int(self.network_config.launch_timeout_seconds * 1000),
            }})
            .build()
            .save(self.config.operator_config_patch)
        )

    def _write_xiaoo_config(self, provider_url: str) -> None:
        self._xiaoo_config.write_text(
            "[llm]\n"
            f'provider = "{CASE_CONSTANTS.llm_provider}"\n'
            f'model = "{CASE_CONSTANTS.llm_model}"\n'
            'api_key_env = "ACTRAIL_XIAOO_NETWORK_POLICY_KEY"\n'
            f"api_base = {json.dumps(provider_url)}\n"
            f"max_tokens = {CASE_CONSTANTS.llm_max_tokens}\n"
            f"context_window = {CASE_CONSTANTS.llm_context_window}\n"
            f'reasoning_effort = "{CASE_CONSTANTS.llm_reasoning_effort}"\n',
            encoding="utf-8",
        )

    def _require_load_grant(self) -> None:
        grants = self.loaded_plugin.get("host_grants")
        expected = (
            "network-policy.rules.apply:kind=deny,"
            f"operation=connect,protocol=any,endpoint={self.provider_endpoint}"
        )
        if not isinstance(grants, list) or expected not in grants:
            raise AssertionError(f"loaded plugin missed {expected}: {grants}")

    def _plugin_command(self, argv: list[str]) -> str:
        response = self.api.command(INSTANCE, argv)
        command = response.get("command")
        if not isinstance(command, dict) or command.get("exit_code") != 0:
            raise AssertionError(f"plugin command failed: {response}")
        return str(command.get("stdout", ""))

    def _dry_run(self) -> tuple[str, dict[str, str]]:
        output = self._plugin_command(["rule", "dry-run", self.provider_endpoint])
        fields: dict[str, str] = {}
        for item in output.split():
            if "=" not in item:
                continue
            key, value = item.split("=", 1)
            fields[key] = value
        return output, fields

    def _require_revision(
        self,
        fields: dict[str, str],
        name: str,
        minimum: int,
    ) -> int:
        raw = fields.get(name)
        try:
            revision = int(raw) if raw is not None else -1
        except ValueError as error:
            raise AssertionError(
                f"network dry-run returned bad {name}: {raw!r}"
            ) from error
        if revision < minimum:
            raise AssertionError(
                f"network dry-run returned {name}={revision}, expected >= {minimum}"
            )
        return revision

    def _network_events(self, trace_id: int) -> list[dict[str, Any]]:
        result = self.runtime.run(
            self.runtime.viewer_command(
                "--output-format",
                "json",
                "events",
                "--trace-id",
                str(trace_id),
            ),
            echo=False,
        )
        if result.returncode != 0:
            raise AssertionError(
                f"actrailviewer events exited with {result.returncode}: "
                f"{result.stderr[-CASE_CONSTANTS.command_diagnostic_tail_characters:]}"
            )
        try:
            document = json.loads(result.stdout)
        except json.JSONDecodeError as error:
            raise AssertionError("actrailviewer events returned invalid JSON") from error
        raw_events = document.get("events")
        if not isinstance(raw_events, list):
            raise AssertionError("actrailviewer events returned no events array")
        events: list[dict[str, Any]] = []
        for event in raw_events:
            if not isinstance(event, dict):
                raise AssertionError("actrailviewer returned a non-object event")
            if event.get("variant") != "net":
                continue
            payload = event.get("payload")
            if not isinstance(payload, dict):
                raise AssertionError("actrailviewer returned a net event without payload")
            events.append(event)
        return events

    @staticmethod
    def _endpoint(provider_url: str) -> str:
        parsed = urlsplit(provider_url)
        if parsed.hostname is None or parsed.port is None:
            raise RuntimeError(f"local provider URL has no numeric endpoint: {provider_url}")
        return f"{parsed.hostname}:{parsed.port}"
