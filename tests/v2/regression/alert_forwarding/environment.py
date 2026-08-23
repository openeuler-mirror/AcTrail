from __future__ import annotations

import json
import os
import re
import sqlite3
import time
from pathlib import Path
from typing import Any

from tests.v2.common.actrail_runtime import ActrailRuntime, CommandResult
from tests.v2.common.alert_proxy import (
    AlertForwardingWebControl,
    AlertProxyTestProfile,
    AlertSubscriberClient,
)
from tests.v2.common.core import TestOutput, TestResult, TestStatus

from .config import AlertForwardingRegressionConfig


TRACE_PATTERN = re.compile(r"trace trace-(\d+) entered Active")
PLUGIN_INSTANCE = "alert-forwarding-trigger.v2"
ALERT_CATEGORY = "consecutive_failure"


class AlertForwardingEnvironment:
    def __init__(
        self,
        config: AlertForwardingRegressionConfig,
        output: TestOutput,
    ):
        self.config = config
        bin_dir = (
            config.bin_dir
            if config.bin_dir.is_absolute()
            else config.repo / config.bin_dir
        )
        self.profile = AlertProxyTestProfile.create(
            config.work_dir,
            bin_dir / "actraild-alert-proxy",
            config.subscriber_port,
            config.subscriber_token,
        )
        self._trigger_config = config.work_dir / "trigger-plugin.toml"
        self._write_trigger_config()
        self.runtime = ActrailRuntime.isolated(
            config.repo,
            config.bin_dir,
            config.command_timeout_seconds,
            output,
            config.work_dir,
            alert_forwarding=self.profile.runtime_paths,
        )
        self.web = AlertForwardingWebControl(
            bin_dir / "actrailweb",
            config.operator_config,
            "127.0.0.1",
            config.web_port,
            config.work_dir,
            config.command_timeout_seconds,
        )
        self._subscribers: list[AlertSubscriberClient] = []
        self._plugin_loaded = False
        self._web_started = False

    @property
    def database(self) -> Path:
        return self.config.work_dir / "data" / "actrail.sqlite"

    @property
    def trigger_script(self) -> Path:
        return Path(__file__).parent / "assets" / "trigger-alert.sh"

    def prepare(self) -> None:
        self.runtime.prepare()
        self.web.start(self.config.alert_timeout_seconds)
        self._web_started = True
        self.web.configure(enabled=True, categories=[ALERT_CATEGORY])
        self.profile.require_running()
        self._load_trigger_plugin()

    def connect_subscriber(self, client_id: str) -> AlertSubscriberClient:
        subscriber = AlertSubscriberClient(
            self.profile.subscriber_address,
            self.profile.token,
            client_id,
            self.config.work_dir / f"{client_id}.subscriber.jsonl",
        )
        self._subscribers.append(subscriber)
        try:
            subscriber.connect(self.config.alert_timeout_seconds)
            subscriber.subscribe(
                f"subscribe-{client_id}",
                [ALERT_CATEGORY],
                ["warning"],
                self.config.command_timeout_seconds,
            )
        except Exception:
            subscriber.close()
            raise
        return subscriber

    def configure_forwarding(self, categories: list[str]) -> None:
        self.web.configure(enabled=True, categories=categories)

    def wait_until_disabled(self) -> None:
        last = None
        for _ in range(self.config.drain_attempts):
            last = self.web.config()
            if last.get("enabled") is False:
                return
            time.sleep(self.config.drain_interval_seconds)
        raise AssertionError(f"forwarding did not become disabled after disconnect: {last}")

    def restart_forwarding(self) -> int:
        self.web.configure(enabled=True, categories=[ALERT_CATEGORY])
        return self.profile.require_running()

    def launch_trigger(self, name: str) -> tuple[int, CommandResult]:
        return self.launch_command(name, [self.trigger_script])

    def launch_command(
        self,
        name: str,
        command: list[Path | str],
        *,
        environment: dict[str, str] | None = None,
    ) -> tuple[int, CommandResult]:
        result = self.runtime.run(
            self.runtime.control_command(
                "launch",
                "--name",
                name,
                "--",
                *command,
            ),
            timeout_seconds=self.config.launch_timeout_seconds,
            environment=environment,
        )
        matches = TRACE_PATTERN.findall(result.output)
        if len(matches) != 1:
            raise AssertionError(
                f"launch did not identify exactly one trace: {matches}; "
                f"output={result.output[-3000:]}"
            )
        return int(matches[0]), result

    def stored_alerts(self, trace_id: int) -> list[dict[str, Any]]:
        if not self.database.is_file():
            return []
        with sqlite3.connect(f"file:{self.database}?mode=ro", uri=True) as connection:
            rows = connection.execute(
                """
                SELECT d.kind, d.title, d.severity_code, a.payload_json
                FROM alerts a
                JOIN alert_definitions d
                  ON a.alert_definition_id = d.alert_definition_id
                WHERE a.trace_id = ? AND d.kind = ?
                ORDER BY a.alert_id
                """,
                (trace_id, ALERT_CATEGORY),
            ).fetchall()
        return [
            {
                "category": row[0],
                "title": row[1],
                "severity_code": row[2],
                "payload_json": row[3],
            }
            for row in rows
        ]

    def viewer_actions(self, trace_id: int) -> list[dict[str, Any]]:
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
            raise AssertionError(f"viewer actions failed: {result.output[-2000:]}")
        document = json.loads(result.stdout)
        actions = document.get("actions") if isinstance(document, dict) else None
        if not isinstance(actions, list):
            raise AssertionError("viewer actions returned no actions array")
        return actions

    def cleanup(self) -> TestResult:
        failures: list[str] = []
        for subscriber in self._subscribers:
            subscriber.close()
        self._subscribers.clear()
        if self._plugin_loaded:
            result = self.runtime.run(
                [
                    self.runtime.actraild,
                    "--config",
                    self.config.operator_config,
                    "plugin",
                    "unload",
                    "--instance",
                    PLUGIN_INSTANCE,
                ],
                echo=False,
            )
            if result.returncode != 0:
                failures.append(f"plugin unload: {result.output[-1000:]}")
            self._plugin_loaded = False
        if self._web_started:
            self.web.stop()
            self._web_started = False
        stopped = self.runtime.stop()
        if stopped is not None and stopped.returncode != 0:
            failures.append(f"daemon stop: {stopped.output[-1000:]}")
        try:
            self.profile.terminate()
        except Exception as error:
            failures.append(f"proxy stop: {error}")
        if failures:
            return TestResult(TestStatus.FAILED, "; ".join(failures))
        return TestResult(
            TestStatus.PASSED,
            "subscribers closed, plugin unloaded, daemon stopped, and proxy terminated",
        )

    def _load_trigger_plugin(self) -> None:
        plugin_root = Path(
            os.environ.get("ACTRAIL_PLUGIN_DIR", Path.home() / ".actrail" / "plugins")
        )
        manifest = (
            plugin_root
            / "tool-consecutive-failure-alert"
            / "tool-consecutive-failure-alert.plugin.toml"
        )
        if not manifest.is_file():
            raise RuntimeError(f"installed alert plugin manifest is missing: {manifest}")
        result = self.runtime.run_checked(
            [
                self.runtime.actraild,
                "--config",
                self.config.operator_config,
                "plugin",
                "load",
                "--manifest",
                manifest,
                "--instance",
                PLUGIN_INSTANCE,
                "--plugin-config",
                self._trigger_config,
                "--grant",
                "alert-write",
            ]
        )
        if "loaded instance=" not in result.output:
            raise AssertionError(f"trigger plugin did not become active: {result.output[-2000:]}")
        self._plugin_loaded = True

    def _write_trigger_config(self) -> None:
        self._trigger_config.write_text(
            "[alert]\n"
            "consecutive_failure_threshold = 1\n"
            "cooldown_seconds = 1\n",
            encoding="utf-8",
        )
