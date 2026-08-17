from __future__ import annotations

import json
import os
import re
import selectors
import shlex
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, TextIO

from tests.v2.common.actrail_runtime import ActrailRuntime, CommandResult
from tests.v2.common.core import TestOutput, TestResult, TestStatus
from tests.v2.common.plugin_test_environment import (
    PluginRuntimeSpec,
    PluginTestEnvironment,
)

from .config import ActivityAnomalyConfig


INSTANCE_ID = "activity-anomaly.v2"
PLUGIN_ID = "actrail.activity-anomaly"
TRACE_PATTERN = re.compile(r"trace trace-(\d+) entered Active")


class ActivityAnomalyEnvironment(PluginTestEnvironment):
    def __init__(self, config: ActivityAnomalyConfig, output: TestOutput):
        self._plugin_root = config.work_dir / "plugins"
        self._xiaoo_config = config.work_dir / "xiaoo-config.toml"
        self._provider_process: subprocess.Popen[str] | None = None
        self._provider_stderr: TextIO | None = None
        super().__init__(
            config,
            output,
            operator_config=config.operator_config,
            operator_config_patch=config.operator_config_patch,
            web_host=config.web_host,
            web_port=config.web_port,
            plugin=PluginRuntimeSpec(
                package="activity-anomaly",
                instance_id=INSTANCE_ID,
                plugin_id=PLUGIN_ID,
                runtime="wasm",
            ),
        )

    @property
    def activity_config(self) -> ActivityAnomalyConfig:
        return self.config

    @property
    def database(self) -> Path:
        return self.activity_config.work_dir / "data" / "actrail.sqlite"

    def prepare(self) -> dict[str, Any]:
        self._install_plugin_package()
        ActrailRuntime.write_isolated_operator_config_patch(
            self.activity_config.operator_config_patch,
            self.activity_config.work_dir,
            plugin_directory=self._plugin_root,
        )
        initial = super().prepare()
        self._require_host_grants()
        self._configure_detection()
        return initial

    def start_provider(self, marker: str) -> str:
        if self._provider_process is not None:
            raise RuntimeError("activity provider is already running")
        script = (
            self.activity_config.repo
            / "tests/agent-trace/multi-container-activity-anomaly/tool_provider.py"
        )
        long_script = (
            self.activity_config.repo
            / "examples/plugins/wit-component/activity-anomaly/long-running-command.sh"
        )
        for required in (script, long_script):
            if not required.is_file():
                raise RuntimeError(f"activity E2E asset is missing: {required}")
        long_command = (
            f"/bin/bash {shlex.quote(str(long_script))} 0 "
            f"{self.activity_config.long_command_seconds}"
        )
        self._provider_stderr = (
            self.activity_config.work_dir / "provider.stderr.log"
        ).open("w", encoding="utf-8")
        self._provider_process = subprocess.Popen(
            [
                sys.executable,
                str(script),
                "--bind-host",
                "127.0.0.1",
                "--bind-port",
                "0",
                "--response-marker",
                marker,
                "--long-command",
                long_command,
            ],
            cwd=self.activity_config.repo,
            stdout=subprocess.PIPE,
            stderr=self._provider_stderr,
            text=True,
            bufsize=1,
        )
        base_url = self._wait_for_provider_url()
        self._write_xiaoo_config(base_url)
        return base_url

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
        environment["NO_PROXY"] = "127.0.0.1,localhost"
        environment["no_proxy"] = "127.0.0.1,localhost"
        environment["ACTRAIL_ACTIVITY_ANOMALY_LOCAL_KEY"] = "local-test-key"
        result = self.runtime.run(
            self.runtime.control_command(
                "launch",
                "--name",
                trace_name,
                "--",
                self.activity_config.xiaoo_binary,
                "--cli",
                "run",
                "--config",
                self._xiaoo_config,
                "--tools",
                "bash",
                "--max-turns",
                "3",
                "--prompt",
                "Execute each requested Bash tool, then return the final result.",
            ),
            timeout_seconds=self.activity_config.launch_timeout_seconds,
            environment=environment,
        )
        if result.returncode != 0:
            raise AssertionError(
                f"real Xiaoo exited with {result.returncode}: {result.output[-4000:]}"
            )
        matches = TRACE_PATTERN.findall(result.output)
        if len(matches) != 1:
            raise AssertionError(
                f"real Xiaoo output did not identify exactly one trace: {matches}"
            )
        return int(matches[0]), result

    def viewer_document(self, trace_id: int) -> dict[str, Any]:
        result = self.runtime.run(
            self.runtime.viewer_command(
                "--output-format",
                "json",
                "actions",
                "--trace-id",
                str(trace_id),
            ),
            echo=False,
        )
        if result.returncode != 0:
            raise AssertionError(
                f"viewer actions failed with {result.returncode}: {result.output[-2000:]}"
            )
        try:
            document = json.loads(result.stdout)
        except json.JSONDecodeError as error:
            raise AssertionError("viewer actions returned invalid JSON") from error
        if not isinstance(document, dict):
            raise AssertionError("viewer actions returned non-object JSON")
        return document

    def plugin_status(self) -> dict[str, str]:
        result = self.runtime.run(
            [
                self.runtime.actraild,
                "--config",
                self.activity_config.operator_config,
                "plugin",
                "status",
                "--instance",
                INSTANCE_ID,
            ],
            echo=False,
        )
        if result.returncode != 0:
            raise AssertionError(
                f"plugin status failed with {result.returncode}: {result.output[-2000:]}"
            )
        fields: dict[str, str] = {}
        for token in result.stdout.split():
            if "=" in token:
                key, value = token.split("=", 1)
                fields[key] = value
        return fields

    def cleanup(self) -> TestResult:
        provider_failure = self._stop_provider()
        base = super().cleanup()
        if provider_failure is None:
            return base
        return TestResult(
            TestStatus.COMPOSITE,
            "activity-anomaly environment cleanup",
            {
                "services": base,
                "provider": TestResult(TestStatus.FAILED, provider_failure),
            },
        )

    def _install_plugin_package(self) -> None:
        configured_root = os.environ.get("ACTRAIL_PLUGIN_DIR")
        source_root = (
            Path(configured_root) if configured_root else Path.home() / ".actrail/plugins"
        )
        source = source_root / "activity-anomaly"
        expected = (
            "activity-anomaly.plugin.toml",
            "activity-anomaly.config.json",
            "activity-anomaly.config.v1.schema.json",
            "actrail_activity_anomaly_plugin.wasm",
            "llm-growth.payload.v1.schema.json",
            "command-duration.payload.v1.schema.json",
        )
        missing = [name for name in expected if not (source / name).is_file()]
        if missing:
            raise RuntimeError(
                f"installed activity-anomaly package is incomplete: {missing} in {source}"
            )
        destination = self._plugin_root / "activity-anomaly"
        shutil.copytree(source, destination, dirs_exist_ok=True)

    def _configure_detection(self) -> None:
        candidate = self.current_config()
        for key in ("request_growth", "response_growth"):
            rule = candidate.get(key)
            if not isinstance(rule, dict):
                raise AssertionError(f"plugin config omitted {key}")
            rule["enabled"] = True
            rule["hard_limit_bytes"] = 1
        command = candidate.get("command_duration")
        if not isinstance(command, dict):
            raise AssertionError("plugin config omitted command_duration")
        command["enabled"] = True
        command["maximum_duration_ms"] = self.activity_config.command_threshold_ms
        updated = self.update_config(candidate)
        if updated != candidate:
            raise AssertionError(
                f"plugin config update was not retained: {updated} != {candidate}"
            )

    def _require_host_grants(self) -> None:
        grants = self.loaded_plugin.get("host_grants")
        required = {"trace-activity-read", "alert-write"}
        if not isinstance(grants, list) or not required.issubset(set(grants)):
            raise AssertionError(f"activity plugin host grants are incomplete: {grants}")

    def _wait_for_provider_url(self) -> str:
        process = self._provider_process
        if process is None or process.stdout is None:
            raise RuntimeError("activity provider has no stdout pipe")
        deadline = time.monotonic() + self.activity_config.provider_ready_timeout_seconds
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ)
        try:
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError(
                        f"activity provider exited early with {process.returncode}"
                    )
                remaining = max(0.0, deadline - time.monotonic())
                if not selector.select(min(0.2, remaining)):
                    continue
                line = process.stdout.readline().strip()
                if line.startswith("provider_base_url="):
                    return line.removeprefix("provider_base_url=")
        finally:
            selector.close()
        raise RuntimeError("activity provider did not publish its URL before timeout")

    def _write_xiaoo_config(self, provider_url: str) -> None:
        self._xiaoo_config.write_text(
            "[llm]\n"
            'provider = "deepseek"\n'
            'model = "deepseek-chat"\n'
            'api_key_env = "ACTRAIL_ACTIVITY_ANOMALY_LOCAL_KEY"\n'
            f"api_base = {json.dumps(provider_url)}\n"
            "max_tokens = 128\n"
            "context_window = 32768\n"
            'reasoning_effort = "off"\n',
            encoding="utf-8",
        )

    def _stop_provider(self) -> str | None:
        failures: list[str] = []
        process = self._provider_process
        if process is not None:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)
            if process.returncode not in (0, -15):
                failures.append(f"provider exited with {process.returncode}")
            if process.stdout is not None:
                process.stdout.close()
            self._provider_process = None
        if self._provider_stderr is not None:
            self._provider_stderr.close()
            self._provider_stderr = None
        return "; ".join(failures) or None
