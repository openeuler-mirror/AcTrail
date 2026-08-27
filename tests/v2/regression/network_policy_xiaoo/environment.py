from __future__ import annotations

import errno
import json
import os
import re
import shutil
import time
from dataclasses import replace
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

from tests.v2.common.actrail_runtime import CommandResult
from tests.v2.common.core import TestOutput, TestResult, TestStatus
from tests.v2.common.plugin_test_environment import (
    PluginRuntimeSpec,
    PluginTestEnvironment,
)
from tests.v2.regression.command_policy_xiaoo.provider import LocalXiaooProvider

from .config import NetworkPolicyXiaooConfig


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
            operator_config_patch=config.operator_config_patch,
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
                        "remote_scope": self.provider_endpoint,
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
            "remote": self.provider_endpoint,
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
                    "remote": self.provider_endpoint,
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
            "remote": self.provider_endpoint,
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
        environment["NO_PROXY"] = "127.0.0.1,localhost"
        environment["no_proxy"] = "127.0.0.1,localhost"
        environment["ACTRAIL_XIAOO_NETWORK_POLICY_KEY"] = "local-test-key"
        environment["PATH"] = "/usr/bin"
        result = self.runtime.run(
            self.runtime.control_command(
                "launch",
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
                "3",
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
                    and payload.get("remote") == self.provider_endpoint
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
                    "policy_remote_scope": self.provider_endpoint,
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
        work_dir = self.network_config.work_dir
        paths = {
            name: json.dumps(str(work_dir / relative))
            for name, relative in {
                "socket": "run/control.sock",
                "pid": "run/actraild.pid",
                "log": "log/actraild.log",
                "storage": "data/actrail.sqlite",
                "export": "data/export",
                "tls_sync": "run/tls-sync.sock",
                "plugins": "plugins",
                "rules": "network-control.rules",
            }.items()
        }
        self.network_config.operator_config_patch.write_text(
            "[control]\n"
            f"socket_path = {paths['socket']}\n"
            f"pid_file = {paths['pid']}\n"
            f"log_path = {paths['log']}\n"
            "\n[storage.sqlite]\n"
            f"path = {paths['storage']}\n"
            "\n[storage.retention]\n"
            "enabled = false\n"
            "\n[export.snapshot]\n"
            f"directory = {paths['export']}\n"
            "\n[payload.tls]\n"
            "enabled = false\n"
            f"sync_event_socket_path = {paths['tls_sync']}\n"
            "\n[payload.stdio]\n"
            "enabled = false\n"
            "\n[payload.socket]\n"
            "enabled = false\n"
            "\n[capture]\n"
            'profile_name = "network-policy-xiaoo-regression"\n'
            'capabilities = ["proc-exec-context", "enforcement-network-connect-seccomp"]\n'
            "\n[ebpf]\n"
            "enabled = false\n"
            "\n[seccomp_notify]\n"
            "enabled = true\n"
            "reserved_listener_fd = 253\n"
            "\n[process_seccomp]\n"
            "enabled = true\n"
            "\n[network_control]\n"
            "enabled = true\n"
            f"rules_path = {paths['rules']}\n"
            'syscalls = ["connect"]\n'
            'default_decision = "allow"\n'
            'failure_decision = "deny"\n'
            "audit_enabled = true\n"
            "audit_default_allow = false\n"
            "pending_decision_max = 64\n"
            "reusable_cache_max_entries = 4096\n"
            "\n[plugins.discovery]\n"
            f"directory = {paths['plugins']}\n"
            "max_packages = 4\n"
            "manifest_max_bytes = 262144\n"
            "\n[plugins.startup]\n"
            "enabled = false\n"
            'failure_policy = "fail-fast"\n',
            encoding="utf-8",
        )

    def _write_xiaoo_config(self, provider_url: str) -> None:
        self._xiaoo_config.write_text(
            "[llm]\n"
            'provider = "deepseek"\n'
            'model = "deepseek-chat"\n'
            'api_key_env = "ACTRAIL_XIAOO_NETWORK_POLICY_KEY"\n'
            f"api_base = {json.dumps(provider_url)}\n"
            "max_tokens = 128\n"
            "context_window = 32768\n"
            'reasoning_effort = "off"\n',
            encoding="utf-8",
        )

    def _require_load_grant(self) -> None:
        grants = self.loaded_plugin.get("host_grants")
        expected = (
            "network-policy.rules.apply:kind=deny,"
            f"remote={self.provider_endpoint}"
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
                f"{result.stderr[-2000:]}"
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
