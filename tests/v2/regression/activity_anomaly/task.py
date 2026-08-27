from __future__ import annotations

import json
import secrets
import sqlite3
import time
from typing import Any

from tests.v2.common.core import TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton

from .environment import ActivityAnomalyEnvironment, PLUGIN_ID


EXPECTED_DEFINITIONS = {
    "llm-request-growth": ("llm.request.growth", "llm.request", "request"),
    "llm-response-growth": ("llm.response.growth", "llm.response", "response"),
    "command-duration-exceeded": (
        "command.duration.exceeded",
        "command.invocation",
        None,
    ),
}
TERMINAL_STATES = frozenset({"exited", "completed"})


class ActivityAnomalyTask:
    def __init__(
        self,
        environment: ActivityAnomalyEnvironment,
        test_context: TestingContextSingleton,
    ):
        self._environment = environment
        self._test_context = test_context

    def run(self) -> dict[str, TestResult]:
        marker = f"ACTRAIL_ACTIVITY_RESPONSE_{secrets.token_hex(8).upper()}"
        trace_name = f"activity-anomaly-{secrets.token_hex(4)}"
        self._test_context.report_progress(
            "real_agent",
            "running deterministic three-turn activity through real Xiaoo",
        )
        self._environment.start_provider(marker)
        trace_id, launch = self._environment.run_xiaoo(trace_name)
        if marker not in launch.output:
            raise AssertionError("real Xiaoo output omitted the provider final marker")
        self._wait_terminal(trace_id)
        document = self._wait_for_scenario(trace_id, marker)
        results = {
            "real-agent-scenario": TestResult(
                TestStatus.PASSED,
                f"trace-{trace_id} captured three real LLM turns and two Bash tools",
            )
        }

        self._test_context.report_progress(
            "alerts",
            "waiting for exactly one alert from each activity rule",
        )
        api_alerts, database_alerts = self._wait_for_stable_alerts(trace_id)
        results["three-alerts"] = TestResult(
            TestStatus.PASSED,
            "request growth, response growth, and command duration each alerted once",
        )

        self._test_context.report_progress(
            "evidence",
            "checking every alert finding against captured actions and LLM links",
        )
        self._assert_api_database_match(api_alerts, database_alerts)
        self._assert_alert_evidence(trace_id, trace_name, document, api_alerts)
        results["real-action-evidence"] = TestResult(
            TestStatus.PASSED,
            "all alert findings resolve to real captured actions and call links",
        )

        self._assert_plugin_health()
        results["plugin-health"] = TestResult(
            TestStatus.PASSED,
            "plugin is active, consumed records, and reports no runtime error",
        )
        return results

    def _wait_terminal(self, trace_id: int) -> None:
        deadline = time.monotonic() + self._environment.activity_config.alert_timeout_seconds
        last_state = "<missing>"
        while time.monotonic() < deadline:
            if self._environment.database.is_file():
                with sqlite3.connect(
                    f"file:{self._environment.database}?mode=ro", uri=True
                ) as connection:
                    row = connection.execute(
                        "SELECT lifecycle_state FROM traces WHERE trace_id = ?",
                        (trace_id,),
                    ).fetchone()
                if row:
                    last_state = str(row[0])
                    if last_state in TERMINAL_STATES:
                        return
                    if last_state == "failed":
                        raise AssertionError(f"trace-{trace_id} entered failed state")
            time.sleep(self._environment.activity_config.drain_interval_seconds)
        raise AssertionError(
            f"trace-{trace_id} did not become terminal; last_state={last_state}"
        )

    def _wait_for_scenario(self, trace_id: int, marker: str) -> dict[str, Any]:
        deadline = time.monotonic() + self._environment.activity_config.alert_timeout_seconds
        last_error = "viewer had no actions"
        while time.monotonic() < deadline:
            try:
                document = self._environment.viewer_document(trace_id)
                self._assert_scenario(document, marker)
                return document
            except AssertionError as error:
                last_error = str(error)
            time.sleep(self._environment.activity_config.drain_interval_seconds)
        raise AssertionError(f"captured Xiaoo scenario was incomplete: {last_error}")

    def _assert_scenario(self, document: dict[str, Any], marker: str) -> None:
        actions = self._object_list(document, "actions")
        links = self._object_list(document, "links")
        kinds = [action.get("kind") for action in actions]
        for kind, minimum in (
            ("llm.call", 3),
            ("llm.request", 3),
            ("llm.response", 3),
            ("command.invocation", 2),
        ):
            count = kinds.count(kind)
            if count < minimum:
                raise AssertionError(f"expected at least {minimum} {kind}, found {count}")
        by_id = self._actions_by_id(actions)
        request_calls: set[str] = set()
        response_calls: set[str] = set()
        for link in links:
            if link.get("valid") is not True:
                continue
            parent = link.get("parent_action_id")
            child = link.get("child_action_id")
            if not isinstance(parent, str) or not isinstance(child, str):
                continue
            if link.get("role") == "llm.call.request" and self._kind(by_id, child) == "llm.request":
                request_calls.add(parent)
            if link.get("role") == "llm.call.response" and self._kind(by_id, child) == "llm.response":
                response_calls.add(parent)
        complete_calls = {
            action_id
            for action_id in request_calls & response_calls
            if self._kind(by_id, action_id) == "llm.call"
        }
        if not complete_calls:
            raise AssertionError(
                "captured LLM actions have no complete call/request/response link"
            )
        serialized = json.dumps(actions, ensure_ascii=False)
        for evidence in (marker, "ACTRAIL_ACTIVITY_WARMUP", "long-running-command.sh"):
            if evidence not in serialized:
                raise AssertionError(f"captured actions omitted scenario evidence {evidence}")

    def _wait_for_stable_alerts(
        self,
        trace_id: int,
    ) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
        deadline = time.monotonic() + self._environment.activity_config.alert_timeout_seconds
        stable_since: float | None = None
        last_keys: list[str] = []
        while time.monotonic() < deadline:
            api_alerts = self._api_activity_alerts(trace_id)
            database_alerts = self._database_activity_alerts(trace_id)
            api_keys = [str(alert.get("definition_key")) for alert in api_alerts]
            database_keys = [str(alert["definition_key"]) for alert in database_alerts]
            last_keys = database_keys
            if len(api_alerts) > 3 or len(database_alerts) > 3:
                raise AssertionError(
                    f"activity alerts were duplicated: api={api_keys} db={database_keys}"
                )
            expected = set(EXPECTED_DEFINITIONS)
            complete = (
                len(api_alerts) == 3
                and len(database_alerts) == 3
                and set(api_keys) == expected
                and set(database_keys) == expected
            )
            if complete:
                stable_since = stable_since or time.monotonic()
                if time.monotonic() - stable_since >= 1.0:
                    return api_alerts, database_alerts
            else:
                stable_since = None
            time.sleep(min(0.2, self._environment.activity_config.drain_interval_seconds))
        raise AssertionError(
            f"expected three stable activity alerts for trace-{trace_id}, found {last_keys}"
        )

    def _api_activity_alerts(self, trace_id: int) -> list[dict[str, Any]]:
        raw = self._environment.api.alerts(trace_id).get("alerts")
        if not isinstance(raw, list):
            raise AssertionError("alerts API returned no alerts array")
        return [
            alert
            for alert in raw
            if isinstance(alert, dict)
            and alert.get("producer_plugin_id") == PLUGIN_ID
        ]

    def _database_activity_alerts(self, trace_id: int) -> list[dict[str, Any]]:
        if not self._environment.database.is_file():
            return []
        with sqlite3.connect(
            f"file:{self._environment.database}?mode=ro", uri=True
        ) as connection:
            rows = connection.execute(
                """
                SELECT a.alert_id, d.definition_key, d.kind, a.payload_json
                FROM alerts a
                JOIN alert_definitions d
                  ON d.alert_definition_id = a.alert_definition_id
                WHERE a.trace_id = ? AND d.producer_plugin_id = ?
                ORDER BY a.alert_id
                """,
                (trace_id, PLUGIN_ID),
            ).fetchall()
        return [
            {
                "alert_id": row[0],
                "definition_key": row[1],
                "kind": row[2],
                "payload": json.loads(row[3]),
            }
            for row in rows
        ]

    def _assert_api_database_match(
        self,
        api_alerts: list[dict[str, Any]],
        database_alerts: list[dict[str, Any]],
    ) -> None:
        api_by_key = {str(alert.get("definition_key")): alert for alert in api_alerts}
        database_by_key = {
            str(alert["definition_key"]): alert for alert in database_alerts
        }
        if set(api_by_key) != set(EXPECTED_DEFINITIONS) or set(database_by_key) != set(
            EXPECTED_DEFINITIONS
        ):
            raise AssertionError("API/database activity alert keys differ")
        for key, (kind, _action_kind, _direction) in EXPECTED_DEFINITIONS.items():
            api_alert = api_by_key[key]
            database_alert = database_by_key[key]
            if api_alert.get("kind") != kind or database_alert.get("kind") != kind:
                raise AssertionError(f"{key} alert kind mismatch")
            if api_alert.get("payload") != database_alert.get("payload"):
                raise AssertionError(f"{key} API payload differs from persistence")

    def _assert_alert_evidence(
        self,
        trace_id: int,
        trace_name: str,
        document: dict[str, Any],
        alerts: list[dict[str, Any]],
    ) -> None:
        actions = self._object_list(document, "actions")
        links = self._object_list(document, "links")
        by_id = self._actions_by_id(actions)
        link_triples = {
            (
                link.get("parent_action_id"),
                link.get("child_action_id"),
                link.get("role"),
            )
            for link in links
            if link.get("valid") is True
        }
        for alert in alerts:
            key = str(alert.get("definition_key"))
            expected_kind, action_kind, direction = EXPECTED_DEFINITIONS[key]
            if alert.get("kind") != expected_kind or alert.get("severity") != "medium":
                raise AssertionError(f"{key} alert metadata mismatch: {alert}")
            payload = alert.get("payload")
            if not isinstance(payload, dict):
                raise AssertionError(f"{key} payload is not an object")
            if payload.get("display_name") != trace_name:
                raise AssertionError(f"{key} payload refers to the wrong trace")
            findings = payload.get("findings")
            if not isinstance(findings, list) or not findings:
                raise AssertionError(f"{key} payload has no findings")
            if payload.get("truncated_count") != 0:
                raise AssertionError(f"{key} findings were unexpectedly truncated")
            if direction is not None:
                if payload.get("direction") != direction or payload.get("hard_limit_bytes") != 1:
                    raise AssertionError(f"{key} did not use configured hard-limit rule")
                link_role = f"llm.call.{direction}"
                for finding in findings:
                    action_id = finding.get("action_id")
                    call_id = finding.get("call_action_id")
                    if self._kind(by_id, action_id) != action_kind:
                        raise AssertionError(f"{key} finding action is not {action_kind}")
                    if self._kind(by_id, call_id) != "llm.call":
                        raise AssertionError(f"{key} finding has no real llm.call")
                    if (call_id, action_id, link_role) not in link_triples:
                        raise AssertionError(f"{key} finding is not linked to its llm.call")
                    if finding.get("reason") != "hard-limit" or int(
                        finding.get("observed_bytes", 0)
                    ) < 1:
                        raise AssertionError(f"{key} finding has no real payload size")
            else:
                self._assert_command_findings(payload, findings, by_id)
        self._assert_trace_identity(trace_id, alerts)

    def _assert_command_findings(
        self,
        payload: dict[str, Any],
        findings: list[dict[str, Any]],
        by_id: dict[str, dict[str, Any]],
    ) -> None:
        threshold = self._environment.activity_config.command_threshold_ms
        if payload.get("maximum_duration_ms") != threshold:
            raise AssertionError("command alert did not retain configured threshold")
        matched_long_command = False
        for finding in findings:
            action_id = finding.get("action_id")
            if self._kind(by_id, action_id) != "command.invocation":
                raise AssertionError("command finding does not resolve to command.invocation")
            agent_action_id = finding.get("agent_action_id")
            if not isinstance(agent_action_id, str) or agent_action_id not in by_id:
                raise AssertionError("command finding has no real Agent action attribution")
            if int(finding.get("duration_ms", 0)) <= threshold:
                raise AssertionError("command finding duration did not exceed threshold")
            if "long-running-command.sh" in str(finding.get("command_line")):
                matched_long_command = True
        if not matched_long_command:
            raise AssertionError("command alert did not identify the provider long command")

    def _assert_trace_identity(
        self,
        trace_id: int,
        alerts: list[dict[str, Any]],
    ) -> None:
        with sqlite3.connect(
            f"file:{self._environment.database}?mode=ro", uri=True
        ) as connection:
            row = connection.execute(
                "SELECT display_name, profile_name FROM traces WHERE trace_id = ?",
                (trace_id,),
            ).fetchone()
        if row is None:
            raise AssertionError(f"trace-{trace_id} disappeared from storage")
        expected_trace = tuple(str(value) for value in row)
        root_process_ids: set[str] = set()
        for alert in alerts:
            payload = alert["payload"]
            root_process_id = payload.get("root_process_id")
            if not isinstance(root_process_id, str) or not root_process_id:
                raise AssertionError(
                    f"{alert.get('definition_key')} has no semantic root process ID"
                )
            root_process_ids.add(root_process_id)
            actual_trace = (
                str(payload.get("display_name")),
                str(payload.get("profile_name")),
            )
            if actual_trace != expected_trace:
                raise AssertionError(
                    f"{alert.get('definition_key')} trace attribution mismatch: "
                    f"{actual_trace} != {expected_trace}"
                )
        if len(root_process_ids) != 1:
            raise AssertionError(
                f"activity alerts disagree on semantic root process: {root_process_ids}"
            )

    def _assert_plugin_health(self) -> None:
        status = self._environment.plugin_status()
        if status.get("state") != "active" or status.get("last_error") != "none":
            raise AssertionError(f"activity plugin is unhealthy: {status}")
        raw_observed = status.get("observed_records", status.get("observed", ""))
        try:
            observed = int(raw_observed)
        except ValueError as error:
            raise AssertionError(f"plugin observed count is invalid: {status}") from error
        if observed <= 0:
            raise AssertionError(f"activity plugin observed no records: {status}")

    @staticmethod
    def _object_list(document: dict[str, Any], key: str) -> list[dict[str, Any]]:
        values = document.get(key)
        if not isinstance(values, list) or not all(
            isinstance(value, dict) for value in values
        ):
            raise AssertionError(f"viewer document has no object array {key}")
        return values

    @staticmethod
    def _actions_by_id(actions: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
        return {
            action_id: action
            for action in actions
            if isinstance((action_id := action.get("action_id")), str)
        }

    @staticmethod
    def _kind(by_id: dict[str, dict[str, Any]], action_id: Any) -> Any:
        if not isinstance(action_id, str):
            return None
        return by_id.get(action_id, {}).get("kind")
