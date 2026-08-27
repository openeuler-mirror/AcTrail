from __future__ import annotations

import json
import re
import secrets
import time
import uuid
from typing import Any

from tests.v2.common.agent_selection import AgentSelector
from tests.v2.common.core import TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton

from .environment import ALERT_CATEGORY, AlertForwardingEnvironment


class AlertForwardingScenario:
    def __init__(
        self,
        environment: AlertForwardingEnvironment,
        test_context: TestingContextSingleton,
    ):
        self._environment = environment
        self._test_context = test_context

    def run(self) -> dict[str, TestResult]:
        self._test_context.report_progress(
            "subscriber_fanout",
            "connecting two authenticated external subscribers",
        )
        first = self._environment.connect_subscriber("alert-e2e-a")
        second = self._environment.connect_subscriber("alert-e2e-b")
        first.wait_for_heartbeat(self._environment.config.alert_timeout_seconds)
        second.wait_for_heartbeat(self._environment.config.alert_timeout_seconds)

        self._test_context.report_progress(
            "script_trigger",
            "running the real failing-command workload at threshold one",
        )
        marker = "ALERT_FORWARDING_SCRIPT_" + secrets.token_hex(6)
        trace_id, _ = self._environment.launch_trigger(marker)
        predicate = lambda alert: alert.get("source", {}).get("trid") == f"trace-{trace_id}"
        first_alert = first.wait_for_alert(
            self._environment.config.alert_timeout_seconds,
            predicate,
        )
        second_alert = second.wait_for_alert(
            self._environment.config.alert_timeout_seconds,
            predicate,
        )
        self._assert_external_alert(first_alert, trace_id)
        self._assert_external_alert(second_alert, trace_id)
        self._assert_stored_source(trace_id, first_alert)
        results = {
            "auto-launch-and-handshake": TestResult(
                TestStatus.PASSED,
                "daemon auto-launched the proxy and both subscribers completed v1 setup",
            ),
            "script-alert-forwarded": TestResult(
                TestStatus.PASSED,
                f"trace-{trace_id} crossed threshold one and reached both subscribers",
            ),
            "storage-forwarding-match": TestResult(
                TestStatus.PASSED,
                "stored and forwarded alerts share trace, category, title, and stable payload",
            ),
        }

        self._test_context.report_progress(
            "category_filter",
            "retaining the stored alert while excluding it from forwarding",
        )
        self._environment.configure_forwarding(["unmatched.e2e.category"])
        filtered_trace, _ = self._environment.launch_trigger(
            "ALERT_FORWARDING_FILTERED_" + secrets.token_hex(6)
        )
        self._wait_for_stored(filtered_trace)
        first.assert_no_alert(self._environment.config.negative_window_seconds)
        results["category-filter"] = TestResult(
            TestStatus.PASSED,
            f"trace-{filtered_trace} was stored but not forwarded by the category filter",
        )

        self._test_context.report_progress(
            "disconnect_recovery",
            "forcing proxy disconnect, observing disable, and enabling a fresh proxy",
        )
        previous_pid = self._environment.profile.require_running()
        self._environment.profile.terminate()
        self._environment.wait_until_disabled()
        first.close()
        second.close()
        recovered_pid = self._environment.restart_forwarding()
        if recovered_pid == previous_pid:
            raise AssertionError("forwarding restart did not create a fresh proxy process")
        recovered = self._environment.connect_subscriber("alert-e2e-recovered")
        recovered.wait_for_heartbeat(self._environment.config.alert_timeout_seconds)
        recovery_trace, _ = self._environment.launch_trigger(
            "ALERT_FORWARDING_RECOVERY_" + secrets.token_hex(6)
        )
        recovery_alert = recovered.wait_for_alert(
            self._environment.config.alert_timeout_seconds,
            lambda value: value.get("source", {}).get("trid")
            == f"trace-{recovery_trace}",
        )
        self._assert_external_alert(recovery_alert, recovery_trace)
        results["disconnect-disable-reenable"] = TestResult(
            TestStatus.PASSED,
            "disconnect disabled forwarding and Web enable auto-launched a fresh proxy",
        )

        self._test_context.report_progress(
            "real_agent",
            "running an alert-producing command through a real agent when available",
        )
        results["real-agent-alert"] = self._run_real_agent(recovered)
        return results

    def _run_real_agent(self, subscriber) -> TestResult:
        agent = AgentSelector(self._environment.config.repo).select(
            self._test_context,
            kinds=("opencode", "codex"),
        )
        if agent is None:
            return TestResult(
                TestStatus.SKIPPED,
                "no usable tool-capable real agent in opencode/codex",
            )
        missing = f"/actrail-alert-agent-{secrets.token_hex(6)}"
        prompt = (
            "Execute exactly this shell command once, then report its result: "
            f"ls {missing}"
        )
        trace_id, _ = self._environment.launch_command(
            "ALERT_FORWARDING_AGENT_" + secrets.token_hex(6),
            agent.command(prompt),
            environment=agent.environment,
        )
        actions = self._environment.viewer_actions(trace_id)
        if not self._agent_executed_command(actions, missing):
            return TestResult(
                TestStatus.SKIPPED,
                f"real agent {agent.kind} did not execute the requested failing command",
            )
        alert = subscriber.wait_for_alert(
            self._environment.config.alert_timeout_seconds,
            lambda value: (
                value.get("source", {}).get("trid") == f"trace-{trace_id}"
                and value.get("extras", {}).get("tool_name") == "ls"
            ),
        )
        self._assert_external_alert(alert, trace_id)
        return TestResult(
            TestStatus.PASSED,
            f"real agent {agent.kind} trace-{trace_id} produced a forwarded alert",
        )

    @staticmethod
    def _agent_executed_command(
        actions: list[dict[str, Any]],
        missing_path: str,
    ) -> bool:
        for action in actions:
            if not isinstance(action, dict):
                continue
            if action.get("kind") != "command.invocation":
                continue
            attributes = action.get("attributes") or {}
            executable = str(attributes.get("process.executable", ""))
            command_line = str(attributes.get("command.line", ""))
            if executable.endswith("/ls") and missing_path in command_line:
                return True
        return False

    def _wait_for_stored(self, trace_id: int) -> list[dict[str, Any]]:
        deadline = time.monotonic() + self._environment.config.alert_timeout_seconds
        while time.monotonic() < deadline:
            stored = self._environment.stored_alerts(trace_id)
            if stored:
                return stored
            time.sleep(self._environment.config.drain_interval_seconds)
        raise AssertionError(f"trace-{trace_id} has no stored alert")

    def _assert_stored_source(self, trace_id: int, external: dict[str, Any]) -> None:
        stored = self._wait_for_stored(trace_id)
        extras = external["extras"]
        for alert in stored:
            payload = json.loads(alert["payload_json"])
            stable_payload = dict(payload)
            stable_extras = dict(extras)
            stable_payload.pop("timestamp", None)
            stable_extras.pop("timestamp", None)
            if (
                alert["category"] == external["cat"]
                and alert["title"] == external["description"]
                and alert["severity_code"] == 4
                and stable_payload == stable_extras
            ):
                return
        raise AssertionError(
            f"trace-{trace_id} stored alerts do not match external alert: {stored}"
        )

    @staticmethod
    def _assert_external_alert(alert: dict[str, Any], trace_id: int) -> None:
        required = {
            "id",
            "ts",
            "source",
            "s",
            "cat",
            "description",
            "labels",
            "extras",
        }
        if set(alert) != required:
            raise AssertionError(f"external alert skeleton mismatch: {alert}")
        try:
            uuid.UUID(str(alert["id"]))
        except ValueError as error:
            raise AssertionError(f"external alert id is not a UUID: {alert}") from error
        if not isinstance(alert["ts"], int) or not re.fullmatch(
            r"\d{13}", str(alert["ts"])
        ):
            raise AssertionError(f"external alert ts is not epoch milliseconds: {alert}")
        expected = {
            "source": {"trid": f"trace-{trace_id}"},
            "s": "warning",
            "cat": ALERT_CATEGORY,
            "description": "同一工具连续失败告警",
            "labels": {},
        }
        for key, value in expected.items():
            if alert.get(key) != value:
                raise AssertionError(
                    f"external alert {key}={alert.get(key)!r}, expected {value!r}"
                )
        if not isinstance(alert["extras"], dict) or not alert["extras"]:
            raise AssertionError(f"external alert extras must be a nonempty object: {alert}")
        evidence = alert["extras"].get("evidence_action_ids")
        if not isinstance(evidence, list) or not any(
            isinstance(action_id, str) and action_id.endswith(":process.exit")
            for action_id in evidence
        ):
            raise AssertionError(f"external alert has no process.exit evidence: {alert}")
