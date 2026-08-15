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

from .environment import ToolFrequentFailureAlertEnvironment

_TRACE_PATTERN = re.compile(r"trace trace-(\d+) entered Active")

_TERMINAL_STATES = frozenset({"exited", "completed"})


class ToolFrequentFailureAlertTask:
    _MISSING_PATHS = (
        "/actrail-missing-frequent-a",
        "/actrail-missing-frequent-b",
        "/actrail-missing-frequent-c",
    )
    _INSUFFICIENT_PATHS = (
        "/actrail-missing-insufficient-a",
        "/actrail-missing-insufficient-b",
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
        "producer_plugin_id": "actrail.tool-frequent-failure-alert",
        "definition_key": "frequent-failure",
        "kind": "frequent_failure",
        "title": "工具频繁失败告警",
        "severity_code": 4,
        "payload_schema_id": "tool-frequent-failure-alert.v1",
    }
    _INSTALLED_FILES = (
        "tool-frequent-failure-alert.plugin.toml",
        "actrail_tool_frequent_failure_alert.wasm",
        "frequent-failure-alert-v1.schema.json",
        "indeterminate-result-v1.schema.json",
    )

    def __init__(
        self,
        environment: ToolFrequentFailureAlertEnvironment,
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
            "direct_mixed_round",
            "three successful and three failing ls commands must alert",
        )
        mixed_trace_id, payload = self._run_positive_round(
            "MIXED",
            self._MISSING_PATHS,
            successful_paths=("/etc/hostname",) * 3,
        )
        results["direct-mixed-alert-persisted"] = TestResult(
            TestStatus.PASSED,
            f"trace-{mixed_trace_id} persisted one mixed-outcome alert: "
            "failures=3 total=6 rate=0.5",
        )

        self._test_context.report_progress(
            "definition",
            "verifying alert definition registration",
        )
        self._assert_definition()
        results["direct-alert-definition"] = TestResult(
            TestStatus.PASSED,
            "frequent-failure definition is registered for the plugin",
        )

        self._test_context.report_progress(
            "payload",
            "verifying frequent-failure alert payload",
        )
        self._assert_payload(
            payload,
            expected_total_count=6,
            expected_failure_rate=0.5,
        )
        results["direct-alert-payload"] = TestResult(
            TestStatus.PASSED,
            "payload reports failures=3 total=6 rate=0.5 for tool=ls",
        )

        self._test_context.report_progress(
            "evidence",
            "verifying unique alert evidence references commands and exits",
        )
        self._assert_evidence(payload)
        results["direct-alert-evidence"] = TestResult(
            TestStatus.PASSED,
            "alert evidence has unique command/exit action ids",
        )

        self._test_context.report_progress(
            "direct_insufficient_round",
            "failures below the window threshold must not alert",
        )
        results["direct-no-alert-below-threshold"] = self._run_no_alert_round(
            "INSUFFICIENT",
            ["bash", "-c", "; ".join(f"ls {p}" for p in self._INSUFFICIENT_PATHS)],
        )

        self._test_context.report_progress(
            "direct_cooldown_round",
            "failures beyond threshold must not repeat alerts",
        )
        results["direct-cooldown-single-alert"] = self._run_cooldown_round()

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
            "direct_installed_package_round",
            "loading the plugin from the installed package",
        )
        results["direct-installed-package-round"] = self._run_installed_package_round()

        self._test_context.report_progress(
            "real_agent",
            "running the unified failing-command prompt through a real agent",
        )
        results["real-agent"] = self._run_real_agent_round()
        return results

    def _check_installed_package(self) -> TestResult:
        package = self._environment.installed_package_dir
        missing = [name for name in self._INSTALLED_FILES if not (package / name).is_file()]
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
        # 同一时刻只保留一个实例，避免两个实例对同一批事件各提交一条告警
        # （宿主去重键本身也保证幂等，这里与参考用例保持一致）。
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
        marker = f"TOOL_FREQUENT_ALERT_{prefix}_" + secrets.token_hex(6)
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
        *,
        successful_paths: tuple[str, ...] = (),
    ) -> tuple[int, dict[str, Any]]:
        marker = f"TOOL_FREQUENT_ALERT_{prefix}_" + secrets.token_hex(6)
        commands = [f"ls {path}" for path in successful_paths]
        commands.extend(f"ls {path}" for path in paths)
        command = ["bash", "-c", "; ".join(commands)]
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
        marker = "TOOL_FREQUENT_ALERT_COOLDOWN_" + secrets.token_hex(6)
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
            self._test_context,
            kinds=("opencode", "codex"),
        )
        if agent is None:
            return TestResult(
                TestStatus.SKIPPED,
                "no usable tool-capable agent binary in opencode/codex",
            )

        # 直接命令回归使用 parent_scope=any；真实 Agent 轮必须切换到
        # llm_and_mcp + agent_child，才能覆盖流式 LLM 工具调用的延迟归因。
        self._environment.unload_repo_plugin()
        try:
            self._environment.load_repo_plugin(
                self._environment.config.agent_plugin_config
            )
            executed_failing_ls = False
            observed_tool_calls = 0
            for attempt in (1, 2):
                marker = (
                    f"TOOL_FREQUENT_ALERT_AGENT_{attempt}_"
                    + secrets.token_hex(6)
                )
                launch = self._launch(
                    marker,
                    agent.command(self._AGENT_PROMPT),
                    environment=agent.environment,
                )
                trace_id = self._require_trace_id(launch)
                self._wait_terminal(trace_id)
                actions = self._environment.viewer_actions(trace_id)
                tool_call_count = self._agent_llm_tool_call_count(actions)
                observed_tool_calls = max(observed_tool_calls, tool_call_count)
                executed = self._agent_executed_failing_ls(actions)
                executed_failing_ls = executed_failing_ls or executed
                alerts = self._poll_alerts(trace_id)
                if len(alerts) == 1:
                    payload = json.loads(alerts[0])
                    if tool_call_count < 3:
                        return TestResult(
                            TestStatus.FAILED,
                            f"trace-{trace_id} alerted but captured only "
                            f"{tool_call_count} LLM tool calls",
                        )
                    if not executed:
                        return TestResult(
                            TestStatus.FAILED,
                            f"trace-{trace_id} alerted without three captured "
                            "failing ls commands",
                        )
                    self._assert_payload(
                        payload,
                        require_tool_name=False,
                        strict_values=False,
                    )
                    self._assert_evidence(payload)
                    evidence_count = len(payload["evidence_action_ids"])
                    payload_json = json.dumps(
                        payload,
                        ensure_ascii=False,
                        sort_keys=True,
                        separators=(",", ":"),
                    )
                    return TestResult(
                        TestStatus.PASSED,
                        f"real agent {agent.kind} trace-{trace_id} attempt "
                        f"{attempt}: tool={payload['tool_name']} "
                        f"failures={payload['failure_count']} "
                        f"total={payload['total_count']} "
                        f"rate={payload['failure_rate']} "
                        f"evidence={evidence_count} "
                        f"captured_llm_tool_calls={tool_call_count}; "
                        f"payload_json={payload_json}",
                    )
                if len(alerts) > 1:
                    return TestResult(
                        TestStatus.FAILED,
                        f"real agent {agent.kind} produced {len(alerts)} alerts "
                        f"for trace-{trace_id}; expected exactly one",
                    )
            if executed_failing_ls:
                return TestResult(
                    TestStatus.FAILED,
                    f"real agent {agent.kind} executed three failing ls "
                    f"commands and captured {observed_tool_calls} LLM tool "
                    "calls but no frequent-failure alert was persisted",
                )
            return TestResult(
                TestStatus.SKIPPED,
                f"real agent {agent.kind} did not execute the three failing "
                "commands; no alert expected",
            )
        finally:
            self._environment.unload_repo_plugin()
            self._environment.load_repo_plugin()

    def _agent_executed_failing_ls(self, actions: list[dict[str, Any]]) -> bool:
        ls_invocations = [
            action
            for action in actions
            if action.get("kind") == "command.invocation"
            and self._is_ls_command(action)
        ]
        return len(ls_invocations) >= 3

    @staticmethod
    def _agent_llm_tool_call_count(actions: list[dict[str, Any]]) -> int:
        count = 0
        for action in actions:
            if action.get("kind") != "llm.response":
                continue
            attributes = action.get("attributes") or {}
            raw = attributes.get("llm.response.tool_calls_json")
            if not isinstance(raw, str) or not raw:
                continue
            try:
                calls = json.loads(raw)
            except json.JSONDecodeError:
                continue
            if isinstance(calls, list):
                count += sum(
                    1 for call in calls if isinstance(call, dict)
                )
        return count

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
        trace_ids = [int(value) for value in _TRACE_PATTERN.findall(launch.output)]
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
                WHERE a.trace_id = ? AND d.definition_key = 'frequent-failure'
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
            f"no frequent-failure alert for trace-{trace_id} "
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
                WHERE producer_plugin_id = 'actrail.tool-frequent-failure-alert'
                  AND definition_key = 'frequent-failure'
                """
            ).fetchall()
        if len(rows) != 1:
            raise AssertionError(
                "expected one frequent-failure definition, "
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
                "frequent-failure definition mismatch: "
                f"actual={actual} expected={self._EXPECTED_DEFINITION}"
            )

    def _assert_payload(
        self,
        payload: dict[str, Any],
        *,
        require_tool_name: bool = True,
        strict_values: bool = True,
        expected_total_count: int = 3,
        expected_failure_rate: float = 1.0,
    ) -> None:
        required = {
            "alert_type",
            "timestamp",
            "trace_id",
            "tool_name",
            "failure_type",
            "exit_status",
            "failure_count",
            "total_count",
            "failure_rate",
            "threshold",
            "window",
            "first_action_id",
            "last_action_id",
            "evidence_action_ids",
            "evidence_truncated_count",
        }
        missing = required.difference(payload)
        if missing:
            raise AssertionError(
                f"alert payload missing keys {sorted(missing)}: {payload}"
            )
        threshold = payload["threshold"]
        window = payload["window"]
        checks = {
            "alert_type": payload["alert_type"] == "frequent_failure",
            "failure_type": payload["failure_type"] == "runtime_error",
            "trace_id": isinstance(payload["trace_id"], str)
            and bool(payload["trace_id"]),
            "threshold": isinstance(threshold, dict)
            and threshold.get("min_failure_count") == 3,
            "window": isinstance(window, dict)
            and isinstance(window.get("start_ms"), int)
            and isinstance(window.get("end_ms"), int)
            and window["start_ms"] > 0
            and window["end_ms"] >= window["start_ms"],
            "summary_absent": "summary" not in payload
            or payload["summary"] == "",
        }
        if strict_values:
            # Shell 矩阵轮：三条 `ls` 以退出码 2 失败；调用方可要求窗口中
            # 同时包含成功的 `ls`，以验证成功计数和失败率分母。
            checks.update(
                {
                    "failure_count": payload["failure_count"] == 3,
                    "total_count": payload["total_count"]
                    == expected_total_count,
                    "failure_rate": payload["failure_rate"]
                    == expected_failure_rate,
                    "exit_status": payload["exit_status"] == "2",
                    "evidence_truncated_count": payload["evidence_truncated_count"]
                    == 0,
                    "failure_breakdown": isinstance(
                        payload.get("failure_breakdown"), list
                    )
                    and sum(
                        item.get("count", 0)
                        for item in payload.get("failure_breakdown", [])
                        if isinstance(item, dict)
                    )
                    == payload["failure_count"],
                }
            )
        else:
            # 真实 Agent 轮：三条顺序失败必须全部归入同一工具窗口。
            checks.update(
                {
                    "tool_name": isinstance(payload["tool_name"], str)
                    and bool(payload["tool_name"]),
                    "failure_count": payload["failure_count"] == 3,
                    "total_count": payload["total_count"] == 3,
                    "failure_rate": payload["failure_rate"] == 1.0,
                    "exit_status": bool(payload["exit_status"]),
                    "evidence_truncated_count": payload["evidence_truncated_count"]
                    == 0,
                }
            )
        if require_tool_name:
            checks["tool_name"] = payload["tool_name"] == "ls"
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
        first = str(payload.get("first_action_id", ""))
        last = str(payload.get("last_action_id", ""))
        checks = {
            "evidence": evidence_ok,
            "first": first.endswith(":command.invocation"),
            "last": last.endswith(":process.exit"),
        }
        failed = [name for name, ok in checks.items() if not ok]
        if failed:
            raise AssertionError(
                f"alert evidence checks failed {sorted(failed)}: {payload}"
            )

    def _plugin_observed_records(self) -> int:
        fields = self._environment.plugin_status()
        raw = fields.get("observed_records", "0")
        try:
            return int(raw)
        except ValueError as error:
            raise AssertionError(
                f"plugin status observed_records is not an integer: {raw!r}"
            ) from error
