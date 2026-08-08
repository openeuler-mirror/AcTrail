from __future__ import annotations

import json
import re
import secrets
import sqlite3
import time
from typing import Any

from tests.v2.common.agent_selection import AgentSelector
from tests.v2.common.core import TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton

from .environment import ToolConsecutiveFailureAlertEnvironment

_TRACE_PATTERN = re.compile(r"trace trace-(\d+) entered Active")

_TERMINAL_STATES = frozenset({"exited", "completed"})


class ToolConsecutiveFailureAlertTask:
    _MISSING_PATHS = (
        "/actrail-missing-consecutive-a",
        "/actrail-missing-consecutive-b",
        "/actrail-missing-consecutive-c",
    )
    _RESET_PATHS = (
        "/actrail-missing-reset-a",
        "/actrail-missing-reset-b",
        "/actrail-missing-reset-c",
        "/actrail-missing-reset-d",
    )
    _COOLDOWN_PATHS = (
        "/actrail-missing-cooldown-a",
        "/actrail-missing-cooldown-b",
        "/actrail-missing-cooldown-c",
        "/actrail-missing-cooldown-d",
        "/actrail-missing-cooldown-e",
    )
    _AGENT_PROMPT = (
        "请分三条独立命令依次执行，每条命令只检查一个路径，不要合并命令，"
        "也不要跳过：ls /actrail-missing-agent-a.txt、"
        "ls /actrail-missing-agent-b.txt、ls /actrail-missing-agent-c.txt。"
        "执行后原样报告每条命令的输出。"
    )
    _EXPECTED_DEFINITION = {
        "producer_plugin_id": "tool-consecutive-failure-alert",
        "definition_key": "consecutive-failure",
        "kind": "consecutive_failure",
        "title": "同一工具连续失败告警",
        "severity_code": 4,
        "payload_schema_id": "consecutive-failure-alert-v1",
    }

    def __init__(
        self,
        environment: ToolConsecutiveFailureAlertEnvironment,
        test_context: TestingContextSingleton,
    ):
        self._environment = environment
        self._test_context = test_context

    def run(self) -> dict[str, TestResult]:
        results: dict[str, TestResult] = {}

        self._test_context.report_progress(
            "installed_package",
            "verifying install-release.sh installed package",
        )
        results["installed-package"] = self._check_installed_package()

        self._test_context.report_progress(
            "success_round",
            "successful external commands must not alert",
        )
        results["no-alert-on-success"] = self._run_no_alert_round(
            "SUCCESS",
            ["bash", "-c", "/bin/true; /bin/true; /bin/true"],
        )

        self._test_context.report_progress(
            "failure_round",
            "three consecutive failing commands must alert",
        )
        failure_trace_id, payload = self._run_positive_round(
            "FAILURE",
            self._MISSING_PATHS,
        )
        results["alert-persisted"] = TestResult(
            TestStatus.PASSED,
            f"trace-{failure_trace_id} persisted exactly one alert",
        )

        self._test_context.report_progress(
            "definition",
            "verifying alert definition registration",
        )
        self._assert_definition()
        results["alert-definition"] = TestResult(
            TestStatus.PASSED,
            "consecutive-failure definition is registered for the plugin",
        )

        self._test_context.report_progress(
            "payload",
            "verifying consecutive-failure alert payload",
        )
        self._assert_payload(payload)
        results["alert-payload"] = TestResult(
            TestStatus.PASSED,
            "payload reports three consecutive ls failures",
        )

        self._test_context.report_progress(
            "evidence",
            "verifying unique alert evidence references process.exit",
        )
        self._assert_evidence(payload)
        results["alert-evidence"] = TestResult(
            TestStatus.PASSED,
            "alert evidence has unique command/exit action ids",
        )

        self._test_context.report_progress(
            "reset_round",
            "a same-tool success between failures must reset the counter",
        )
        reset_command = (
            f"ls {self._RESET_PATHS[0]}; ls {self._RESET_PATHS[1]}; "
            "ls /etc/hostname; "
            f"ls {self._RESET_PATHS[2]}; ls {self._RESET_PATHS[3]}"
        )
        results["no-alert-on-reset"] = self._run_no_alert_round(
            "RESET",
            ["bash", "-c", reset_command],
        )

        self._test_context.report_progress(
            "cooldown_round",
            "failures beyond threshold must not repeat alerts",
        )
        results["cooldown-single-alert"] = self._run_cooldown_round()

        self._test_context.report_progress(
            "plugin_observed",
            "checking plugin consumed observation records",
        )
        observed = self._plugin_observed_records()
        if observed <= 0:
            results["plugin-observed"] = TestResult(
                TestStatus.FAILED,
                f"plugin observed_records={observed}, expected > 0",
            )
        else:
            results["plugin-observed"] = TestResult(
                TestStatus.PASSED,
                f"plugin observed_records={observed}",
            )

        self._test_context.report_progress(
            "installed_round",
            "loading the plugin from the installed package",
        )
        results["installed-package-round"] = self._run_installed_package_round()

        self._test_context.report_progress(
            "real_agent",
            "running the unified failing-command prompt through a real agent",
        )
        results["real-agent"] = self._run_real_agent_round()
        return results

    def _check_installed_package(self) -> TestResult:
        package = self._environment.installed_package_dir
        expected = (
            "tool-consecutive-failure-alert.plugin.toml",
            "actrail_tool_consecutive_failure_alert.wasm",
            "alert-schema.json",
        )
        missing = [name for name in expected if not (package / name).is_file()]
        if missing:
            return TestResult(
                TestStatus.FAILED,
                "install-release.sh package is missing files under "
                f"{package}: {missing}",
            )
        return TestResult(
            TestStatus.PASSED,
            f"install-release.sh installed package at {package}",
        )

    def _run_installed_package_round(self) -> TestResult:
        if not self._environment.installed_manifest.is_file():
            return TestResult(
                TestStatus.FAILED,
                f"installed plugin manifest is missing: "
                f"{self._environment.installed_manifest}",
            )
        # 同一时刻只保留一个实例，避免两个实例对同一批事件各提交一条告警。
        self._environment.unload_repo_plugin()
        try:
            self._environment.load_installed_plugin()
            trace_id, payload = self._run_positive_round(
                "INSTALLED",
                self._MISSING_PATHS,
            )
        finally:
            self._environment.unload_installed_plugin()
            self._environment.load_repo_plugin()
        self._assert_payload(payload)
        self._assert_evidence(payload)
        return TestResult(
            TestStatus.PASSED,
            f"installed package produced an alert for trace-{trace_id}",
        )

    def _run_no_alert_round(
        self,
        prefix: str,
        command: list[str],
    ) -> TestResult:
        marker = f"TOOL_ALERT_{prefix}_" + secrets.token_hex(6)
        launch = self._launch(marker, command)
        trace_id = self._require_trace_id(launch)
        self._wait_terminal(trace_id)
        alerts = self._query_alerts(trace_id)
        if alerts:
            raise AssertionError(
                f"{prefix.lower()} round trace-{trace_id} produced alerts: "
                f"{alerts}"
            )
        return TestResult(
            TestStatus.PASSED,
            f"trace-{trace_id} produced no alerts",
        )

    def _run_positive_round(
        self,
        prefix: str,
        paths: tuple[str, ...],
    ) -> tuple[int, dict[str, Any]]:
        marker = f"TOOL_ALERT_{prefix}_" + secrets.token_hex(6)
        command = ["bash", "-c", "; ".join(f"ls {path}" for path in paths)]
        launch = self._launch(marker, command)
        trace_id = self._require_trace_id(launch)
        self._wait_terminal(trace_id)
        alerts = self._wait_for_alert(trace_id)
        if len(alerts) != 1:
            raise AssertionError(
                f"expected exactly one alert for trace-{trace_id}, "
                f"found {len(alerts)}: {alerts}"
            )
        try:
            payload = json.loads(alerts[0])
        except json.JSONDecodeError as error:
            raise AssertionError(
                f"alert payload is not valid JSON: {alerts[0]}"
            ) from error
        if not isinstance(payload, dict):
            raise AssertionError(
                f"alert payload is not an object: {alerts[0]}"
            )
        return trace_id, payload

    def _run_cooldown_round(self) -> TestResult:
        marker = "TOOL_ALERT_COOLDOWN_" + secrets.token_hex(6)
        command = [
            "bash",
            "-c",
            "; ".join(f"ls {path}" for path in self._COOLDOWN_PATHS),
        ]
        launch = self._launch(marker, command)
        trace_id = self._require_trace_id(launch)
        self._wait_terminal(trace_id)
        alerts = self._wait_for_alert(trace_id)
        if len(alerts) != 1:
            raise AssertionError(
                "cooldown round expected exactly one alert for "
                f"trace-{trace_id}, found {len(alerts)}: {alerts}"
            )
        payload = json.loads(alerts[0])
        self._assert_payload(payload)
        self._assert_evidence(payload)
        # 超过阈值的额外失败必须被冷却抑制：存储的 command.invocation 数量
        # 应明显大于阈值（bash + 每个失败命令各一条）。
        invocations = self._stored_command_invocation_count(trace_id)
        if invocations < len(self._COOLDOWN_PATHS) + 1:
            raise AssertionError(
                "cooldown round stored command.invocation count is "
                f"{invocations}, expected >= {len(self._COOLDOWN_PATHS) + 1}"
            )
        return TestResult(
            TestStatus.PASSED,
            f"trace-{trace_id} kept exactly one alert for "
            f"{len(self._COOLDOWN_PATHS)} failing commands",
        )

    def _stored_command_invocation_count(self, trace_id: int) -> int:
        actions = self._environment.viewer_actions(trace_id)
        return sum(
            1 for action in actions if action.get("kind") == "command.invocation"
        )

    def _run_real_agent_round(self) -> TestResult:
        agent = AgentSelector(self._environment.config.repo).select(
            self._test_context
        )
        if agent is None:
            return TestResult(
                TestStatus.SKIPPED,
                "no usable agent binary in xiaoo/pi/opencode/claude/codex",
            )
        executed_failing_ls = False
        for attempt in (1, 2):
            marker = f"TOOL_ALERT_AGENT_{attempt}_" + secrets.token_hex(6)
            launch = self._launch(
                marker,
                agent.command(self._AGENT_PROMPT),
                environment=agent.environment,
            )
            trace_id = self._require_trace_id(launch)
            self._wait_terminal(trace_id)
            alerts = self._poll_alerts(trace_id)
            if len(alerts) == 1:
                payload = json.loads(alerts[0])
                self._assert_payload(payload)
                self._assert_evidence(payload)
                return TestResult(
                    TestStatus.PASSED,
                    f"real agent {agent.kind} produced a consecutive-failure "
                    f"alert for trace-{trace_id}",
                )
            if len(alerts) > 1:
                raise AssertionError(
                    f"real agent {agent.kind} produced {len(alerts)} alerts "
                    f"for trace-{trace_id}: {alerts}"
                )
            executed = self._agent_executed_failing_ls(trace_id)
            executed_failing_ls = executed_failing_ls or executed
        if executed_failing_ls:
            raise AssertionError(
                f"real agent {agent.kind} executed failing ls commands but "
                "no consecutive-failure alert was persisted"
            )
        return TestResult(
            TestStatus.SKIPPED,
            f"real agent {agent.kind} did not execute the three failing "
            "commands; no alert expected",
        )

    def _agent_executed_failing_ls(self, trace_id: int) -> bool:
        actions = self._environment.viewer_actions(trace_id)
        ls_invocations = [
            action
            for action in actions
            if action.get("kind") == "command.invocation"
            and self._is_ls_command(action)
        ]
        return len(ls_invocations) >= 3

    @staticmethod
    def _is_ls_command(action: dict[str, Any]) -> bool:
        attributes = action.get("attributes") or {}
        executable = str(attributes.get("process.executable", ""))
        line = str(attributes.get("command.line", ""))
        return executable.endswith("/ls") or line.strip().startswith("ls ")

    def _poll_alerts(self, trace_id: int) -> list[str]:
        alerts: list[str] = []
        for _ in range(self._environment.config.drain_attempts):
            alerts = self._query_alerts(trace_id)
            if alerts:
                break
            time.sleep(self._environment.config.drain_interval_seconds)
        return alerts

    def _launch(
        self,
        marker: str,
        command: list[str],
        *,
        environment: dict[str, str] | None = None,
    ):
        return self._environment.runtime.run(
            self._environment.launch_command(marker, command),
            timeout_seconds=self._environment.config.launch_timeout_seconds,
            environment=environment,
        )

    def _require_trace_id(self, launch) -> int:
        trace_ids = [
            int(value)
            for value in _TRACE_PATTERN.findall(launch.output)
        ]
        if len(trace_ids) != 1:
            raise AssertionError(
                f"expected one trace id, found {trace_ids}: "
                f"{launch.output[-4000:]}"
            )
        return trace_ids[0]

    def _wait_terminal(self, trace_id: int) -> None:
        last_state = "<missing>"
        for _ in range(self._environment.config.drain_attempts):
            state = self._trace_state(trace_id)
            if state is None:
                last_state = "<missing>"
            else:
                last_state = state
                if state in _TERMINAL_STATES:
                    return
                if state == "failed":
                    raise AssertionError(
                        f"trace-{trace_id} entered failed lifecycle state"
                    )
            time.sleep(self._environment.config.drain_interval_seconds)
        raise AssertionError(
            f"trace-{trace_id} did not reach a terminal state; "
            f"last={last_state}"
        )

    def _trace_state(self, trace_id: int) -> str | None:
        database = self._environment.database
        if not database.is_file():
            return None
        with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
            row = connection.execute(
                "SELECT lifecycle_state FROM traces WHERE trace_id = ?",
                (trace_id,),
            ).fetchone()
        return row[0] if row else None

    def _query_alerts(self, trace_id: int) -> list[str]:
        database = self._environment.database
        if not database.is_file():
            return []
        with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
            rows = connection.execute(
                """
                SELECT a.payload_json
                FROM alerts a
                JOIN alert_definitions d
                  ON a.alert_definition_id = d.alert_definition_id
                WHERE a.trace_id = ? AND d.definition_key = 'consecutive-failure'
                ORDER BY a.alert_id
                """,
                (trace_id,),
            ).fetchall()
        return [row[0] for row in rows]

    def _wait_for_alert(self, trace_id: int) -> list[str]:
        for _ in range(self._environment.config.drain_attempts):
            alerts = self._query_alerts(trace_id)
            if alerts:
                return alerts
            time.sleep(self._environment.config.drain_interval_seconds)
        raise AssertionError(
            f"no consecutive-failure alert for trace-{trace_id} "
            "within the drain window"
        )

    def _assert_definition(self) -> None:
        database = self._environment.database
        with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
            rows = connection.execute(
                """
                SELECT producer_plugin_id, definition_key, kind, title,
                       severity_code, payload_schema_id
                FROM alert_definitions
                WHERE producer_plugin_id = 'tool-consecutive-failure-alert'
                  AND definition_key = 'consecutive-failure'
                """
            ).fetchall()
        if len(rows) != 1:
            raise AssertionError(
                "expected one consecutive-failure definition, "
                f"found {rows}"
            )
        row = rows[0]
        actual = {
            "producer_plugin_id": row[0],
            "definition_key": row[1],
            "kind": row[2],
            "title": row[3],
            "severity_code": row[4],
            "payload_schema_id": row[5],
        }
        if actual != self._EXPECTED_DEFINITION:
            raise AssertionError(
                "consecutive-failure definition mismatch: "
                f"actual={actual} expected={self._EXPECTED_DEFINITION}"
            )

    def _assert_payload(self, payload: dict[str, Any]) -> None:
        required = {
            "alert_type",
            "timestamp",
            "tool_name",
            "tool_args",
            "consecutive_failures",
            "threshold",
            "failure_summary",
            "failure_sequence",
            "evidence_action_ids",
        }
        missing = required.difference(payload)
        if missing:
            raise AssertionError(
                f"alert payload missing keys {sorted(missing)}: {payload}"
            )
        checks = {
            "alert_type": payload["alert_type"] == "consecutive_failure",
            "consecutive_failures": payload["consecutive_failures"] == 3,
            "threshold": payload["threshold"] == 3,
            "tool_name": payload["tool_name"] == "ls",
            "failure_summary": "exit code" in payload["failure_summary"],
        }
        failed = [name for name, ok in checks.items() if not ok]
        if failed:
            raise AssertionError(
                f"alert payload checks failed {sorted(failed)}: {payload}"
            )

    def _assert_evidence(self, payload: dict[str, Any]) -> None:
        evidence = payload["evidence_action_ids"]
        evidence_ok = (
            isinstance(evidence, list)
            and len(evidence) == len(set(evidence))
            and sum(
                1
                for action_id in evidence
                if isinstance(action_id, str)
                and action_id.endswith(":process.exit")
            )
            >= 3
            and sum(
                1
                for action_id in evidence
                if isinstance(action_id, str)
                and action_id.endswith(":command.invocation")
            )
            >= 3
        )
        sequence = payload["failure_sequence"]
        sequence_ok = (
            isinstance(sequence, dict)
            and str(sequence.get("first_action_id", "")).endswith(
                ":command.invocation"
            )
            and str(sequence.get("last_action_id", "")).endswith(":process.exit")
        )
        checks = {
            "evidence": evidence_ok,
            "sequence": sequence_ok,
        }
        failed = [name for name, ok in checks.items() if not ok]
        if failed:
            raise AssertionError(
                f"alert evidence checks failed {sorted(failed)}: {payload}"
            )

    def _plugin_observed_records(self) -> int:
        status = self._environment.plugin_status()
        raw = status.get("observed_records", "")
        try:
            return int(raw)
        except ValueError as error:
            raise AssertionError(
                "plugin status observed_records field is not an integer: "
                f"{raw!r}"
            ) from error
