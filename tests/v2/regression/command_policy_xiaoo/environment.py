from __future__ import annotations

import json
import os
import re
import shutil
import sys
import time
from pathlib import Path
from typing import Any

from tests.v2.common.actrail_runtime import CommandResult
from tests.v2.common.patch_builder import PatchBuilder
from tests.v2.common.core import TestOutput, TestResult, TestStatus
from tests.v2.common.plugin_test_environment import (
    PluginRuntimeSpec,
    PluginTestEnvironment,
)

from .config import CommandPolicyXiaooConfig
from .provider import LocalXiaooProvider

from .constants import CASE_CONSTANTS


INSTANCE = "wasm.command-policy-dynamic"
RULE_ID = "command-dynamic-1"
TRACE_RE = re.compile(r"trace trace-(\d+) entered Active")


class CommandPolicyXiaooEnvironment(PluginTestEnvironment):
    def __init__(
        self,
        config: CommandPolicyXiaooConfig,
        output: TestOutput,
    ):
        self.marker = config.work_dir / "xiaoo-command.marker"
        self._plugin_root = config.work_dir / "plugins"
        self._rules = config.work_dir / "command-control.rules"
        self._xiaoo_config = config.work_dir / "xiaoo-config.toml"
        self._thread_exec_fixture = (
            config.repo
            / "tests"
            / "v2"
            / "regression"
            / "command_policy_xiaoo"
            / "thread_exec.py"
        )
        self._provider = LocalXiaooProvider(
            config.repo,
            config.work_dir,
            self.marker,
            config.ready_timeout_seconds,
        )
        super().__init__(
            config,
            output,
            operator_config=config.operator_config,
            operator_config_patches=[config.operator_config_patch, Path(__file__).with_name("patch_config.toml")],
            web_host=config.web_host,
            web_port=config.web_port,
            plugin=PluginRuntimeSpec(
                package="command-policy-dynamic",
                instance_id=INSTANCE,
                plugin_id=INSTANCE,
                runtime="wasm",
                load_grants={
                    "command_policy_rules_apply": [
                        {
                            "decision": "deny",
                            "path_scope": str(config.bash_executable),
                        }
                    ]
                },
            ),
        )

    @property
    def command_config(self) -> CommandPolicyXiaooConfig:
        return self.config

    def prepare(self) -> dict[str, Any]:
        self._install_plugin_package()
        self._rules.write_text("", encoding="utf-8")
        self._write_operator_patch()
        provider_url = self._provider.start()
        self._write_xiaoo_config(provider_url)
        initial = super().prepare()
        self._require_load_grant()
        return initial

    def require_atomic_rejection(self) -> str:
        initial = self.current_config()
        if initial != {"rules": []}:
            raise AssertionError(f"publisher did not start empty: {initial}")
        before_revision = self.dry_run_revision()
        candidate = {
            "rules": [
                {
                    "decision": "deny",
                    "executable": str(self.command_config.bash_executable),
                    "args": ["-c", "*"],
                    "priority": CASE_CONSTANTS.priority,
                },
                {
                    "decision": "deny",
                    "executable": "/srv/not-granted-command",
                    "priority": CASE_CONSTANTS.fallback_rule_priority,
                },
            ]
        }
        validation = self.api.validate_config(INSTANCE, candidate)
        errors = validation.get("errors")
        if validation.get("valid") is not False or not isinstance(errors, list):
            raise AssertionError(f"grant overflow Test was not rejected: {validation}")
        if not any(
            "missing command-policy.rules.apply grant" in str(error)
            for error in errors
        ):
            raise AssertionError(f"grant rejection reason is missing: {validation}")
        if self.current_config() != initial:
            raise AssertionError("failed Configuration Test changed plugin memory")
        after_revision = self.dry_run_revision()
        if after_revision != before_revision:
            raise AssertionError(
                "failed Configuration Test changed daemon revision: "
                f"{before_revision} -> {after_revision}"
            )
        return before_revision

    def publish_deny(self) -> str:
        candidate = {
            "rules": [
                {
                    "decision": "deny",
                    "executable": str(self.command_config.bash_executable),
                    "args": ["-c", "*"],
                    "priority": CASE_CONSTANTS.priority,
                }
            ]
        }
        updated = self.update_config(candidate)
        rules = updated.get("rules")
        if not isinstance(rules, list) or len(rules) != 1:
            raise AssertionError(f"command policy update returned bad rules: {updated}")
        rule = rules[0]
        if not isinstance(rule, dict) or rule.get("rule_id") != RULE_ID:
            raise AssertionError(f"command policy rule ID is unstable: {updated}")
        output = self._plugin_command(
            [
                "rule",
                "dry-run",
                str(self.command_config.bash_executable),
                "--args-json",
                '["-c","printf test"]',
            ]
        )
        required = (
            "matched=true",
            "decision=deny",
            f"rule_id={RULE_ID}",
            f"owner={INSTANCE}",
            "rule_revision=",
            "source_revision=",
        )
        if not all(value in output for value in required):
            raise AssertionError(f"published route dry-run is incomplete: {output}")
        return output

    def require_nonmatching_args_allowed(self) -> None:
        result = self.runtime.run(
            self.runtime.control_command(
                "launch",
                "--seccomp-notify",
                "required",
                "--name",
                "v2-command-policy-bash-nonmatching-args",
                "--",
                self.command_config.bash_executable,
                "--version",
            ),
            timeout_seconds=self.command_config.launch_timeout_seconds,
        )
        if result.returncode != 0 or "GNU bash" not in result.output:
            raise AssertionError(
                "same Bash binary with argv outside [-c, *] was not allowed: "
                f"{result.output[-CASE_CONSTANTS.command_diagnostic_tail_characters:]}"
            )

    def require_nonleader_exec_allowed(self) -> None:
        result = self.runtime.run(
            self.runtime.control_command(
                "launch",
                "--seccomp-notify",
                "required",
                "--name",
                "v2-command-policy-nonleader-exec",
                "--",
                Path(sys.executable).resolve(),
                self._thread_exec_fixture,
                self.command_config.bash_executable,
                "--version",
            ),
            timeout_seconds=self.command_config.launch_timeout_seconds,
        )
        if result.returncode != 0 or "GNU bash" not in result.output:
            raise AssertionError(
                "non-leader thread exec outside [-c, *] was not allowed: "
                f"{result.output[-CASE_CONSTANTS.command_diagnostic_tail_characters:]}"
            )

    def run_xiaoo(self, trace_name: str) -> tuple[int, CommandResult]:
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
        environment["ACTRAIL_XIAOO_COMMAND_POLICY_KEY"] = "local-test-key"
        environment["PATH"] = "/usr/bin"
        result = self.runtime.run(
            self.runtime.control_command(
                "launch",
                "--seccomp-notify",
                "required",
                "--name",
                trace_name,
                "--",
                self.command_config.xiaoo_binary,
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
            timeout_seconds=self.command_config.launch_timeout_seconds,
            environment=environment,
        )
        if result.returncode != 0:
            raise AssertionError(
                f"real Xiaoo launch failed with {result.returncode}: {result.output[-CASE_CONSTANTS.launch_diagnostic_tail_characters:]}"
            )
        match = TRACE_RE.search(result.output)
        if match is None:
            raise AssertionError(f"Xiaoo output omitted trace ID: {result.output}")
        if "enforcement-command-execution-seccomp" not in result.output:
            raise AssertionError("launch omitted command enforcement capability")
        return int(match.group(1)), result

    def require_denied(self, result: CommandResult) -> None:
        if self.marker.exists():
            raise AssertionError("denied Xiaoo Bash tool created its marker")
        if not self._permission_error(result.output):
            raise AssertionError(
                f"Xiaoo did not report an OS permission error: {result.output}"
            )

    def wait_for_evidence(self, trace_id: int) -> tuple[str, dict[str, Any]]:
        deadline = time.monotonic() + self.command_config.evidence_timeout_seconds
        viewer_output = ""
        while time.monotonic() < deadline:
            self.runtime.run_checked(self.runtime.control_command("list-traces"))
            viewer_output = self.runtime.run_checked(
                self.runtime.viewer_command("events", "--trace-id", str(trace_id))
            ).output
            alerts = self.api.alerts(trace_id).get("alerts")
            alert = alerts[0] if isinstance(alerts, list) and alerts else None
            viewer_ready = all(
                value in viewer_output
                for value in (
                    "Enforcement",
                    "seccomp-user-notify",
                    str(self.command_config.bash_executable),
                    "denied",
                )
            )
            if viewer_ready and isinstance(alert, dict):
                expected = {
                    "definition_key": "command-execution-boundary-violation",
                    "kind": "command.execution.boundary-violation",
                    "producer_plugin_id": "actraild.enforcement",
                    "severity": "high",
                }
                if any(alert.get(key) != value for key, value in expected.items()):
                    raise AssertionError(f"boundary alert is wrong: {alert}")
                payload = alert.get("payload")
                if (
                    not isinstance(payload, dict)
                    or payload.get("executable")
                    != str(self.command_config.bash_executable)
                    or payload.get("rule_id") != RULE_ID
                ):
                    raise AssertionError(f"boundary alert payload is wrong: {alert}")
                return viewer_output, alert
            time.sleep(self.command_config.drain_interval_seconds)
        raise AssertionError(
            f"Enforcement/alert evidence is incomplete: {viewer_output}"
        )

    def unload_owner(self) -> None:
        response = self.unload_plugin()
        plugin = response.get("plugin")
        if not isinstance(plugin, dict) or plugin.get("state") == "active":
            raise AssertionError(f"policy owner remained active: {response}")

    def require_allowed(self, result: CommandResult) -> None:
        if self._permission_error(result.output):
            raise AssertionError(f"Bash remained denied after unload: {result.output}")
        if not self.marker.is_file():
            raise AssertionError("Bash did not create marker after owner unload")
        marker = self.marker.read_text(encoding="utf-8").strip()
        if marker != "ACTRAIL_XIAOO_COMMAND_OK":
            raise AssertionError(f"marker content is wrong after unload: {marker!r}")

    def cleanup(self) -> TestResult:
        base = super().cleanup()
        provider_failure = self._provider.stop()
        if provider_failure is None:
            return base
        return TestResult(
            TestStatus.COMPOSITE,
            "command-policy environment cleanup",
            {
                "services": base,
                "provider": TestResult(TestStatus.FAILED, provider_failure),
            },
        )

    def _install_plugin_package(self) -> None:
        source = (
            self.command_config.repo
            / "examples/plugins/wit-component/command-policy-dynamic"
        )
        destination = self._plugin_root / "command-policy-dynamic"
        destination.mkdir(parents=True)
        assets = {
            "plugin.toml": "command-policy-dynamic.plugin.toml",
            "command-policy-dynamic.config.json": "command-policy-dynamic.config.json",
            "component-command-policy-dynamic.wasm": "component-command-policy-dynamic.wasm",
            "config.schema.json": "config.schema.json",
        }
        for source_name, destination_name in assets.items():
            source_path = source / source_name
            if not source_path.is_file():
                raise RuntimeError(f"official command-policy asset missing: {source_path}")
            shutil.copy2(source_path, destination / destination_name)

    def _write_operator_patch(self) -> None:
        (
            PatchBuilder()
            .with_work_dir(self.config.work_dir)
            .with_values({"command_control": {"rules_path": str(self._rules)}})
            .build()
            .save(self.config.operator_config_patch)
        )

    def _write_xiaoo_config(self, provider_url: str) -> None:
        self._xiaoo_config.write_text(
            "[llm]\n"
            f'provider = "{CASE_CONSTANTS.llm_provider}"\n'
            f'model = "{CASE_CONSTANTS.llm_model}"\n'
            'api_key_env = "ACTRAIL_XIAOO_COMMAND_POLICY_KEY"\n'
            f"api_base = {json.dumps(provider_url)}\n"
            f"max_tokens = {CASE_CONSTANTS.llm_max_tokens}\n"
            f"context_window = {CASE_CONSTANTS.llm_context_window}\n"
            f'reasoning_effort = "{CASE_CONSTANTS.llm_reasoning_effort}"\n',
            encoding="utf-8",
        )

    def _require_load_grant(self) -> None:
        grants = self.loaded_plugin.get("host_grants")
        expected = (
            "command-policy.rules.apply:kind=deny,path="
            + str(self.command_config.bash_executable)
        )
        if not isinstance(grants, list) or expected not in grants:
            raise AssertionError(f"loaded plugin missed {expected}: {grants}")

    def dry_run_revision(self) -> str:
        output = self._plugin_command(
            [
                "rule",
                "dry-run",
                str(self.command_config.bash_executable),
                "--args-json",
                '["-c","probe"]',
            ]
        )
        for item in output.split():
            if item.startswith("source_revision="):
                return item.split("=", 1)[1]
        raise AssertionError(f"dry-run omitted source revision: {output}")

    def _plugin_command(self, argv: list[str]) -> str:
        response = self.api.command(INSTANCE, argv)
        command = response.get("command")
        if not isinstance(command, dict) or command.get("exit_code") != 0:
            raise AssertionError(f"plugin command failed: {response}")
        return str(command.get("stdout", ""))

    @staticmethod
    def _permission_error(output: str) -> bool:
        lowered = output.lower()
        return "permission denied" in lowered or "operation not permitted" in lowered
