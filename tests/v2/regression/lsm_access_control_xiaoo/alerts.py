from __future__ import annotations

import json
import sqlite3
import time

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core import TestResult, TestStatus


class BoundaryAlertsScenario:
    """Verify real Xiaoo denials, stored alert deltas and audit retention."""

    def __init__(self, case):
        self.case = case

    def run(self, context, results: dict, port: int) -> None:
        case = self.case
        for name, audit, window, capacity in (
            ("alerts_trace_tail", True, 60_000, 256),
            ("alerts_without_audit", False, 60_000, 256),
            ("alerts_periodic", False, 200, 256),
            ("alerts_capacity", False, 60_000, 1),
        ):
            context.report_progress(name, "fresh defaults; real Xiaoo repeats file and command denials")
            case._audit_enabled = audit
            case._alert_window_ms = window
            case._alert_max_groups = capacity
            case._write_patch()
            case._write_rules("deny", port)
            case._runtime = ActrailRuntime(
                case._config.repo, case._config.bin_dir, case._config.command_timeout_seconds,
                context.output, case._work / "actraild.conf", case._patch,
            )
            case._runtime.prepare()
            trace_id = case._run_xiaoo(name)
            case._require_workload("deny")
            workload = json.loads(case._results.read_text(encoding="utf-8"))
            for domain in ("file", "command"):
                if workload[domain]["attempts"] != 64 or workload[domain]["denials"] != 64:
                    raise AssertionError(f"real {domain} workload did not produce all denials: {workload}")
            alerts = self._wait_for_alerts(trace_id, capacity)
            self._require_alerts(alerts, capacity, periodic=window == 200)
            if audit:
                case._require_evidence(trace_id, "deny")
            stopped = case._runtime.stop()
            if stopped is not None and stopped.returncode != 0:
                raise RuntimeError(f"daemon stop failed: {stopped.output}")
            stored = self._read_alerts(trace_id)
            if stored != alerts:
                raise AssertionError("shutdown changed already-flushed trace alert counts")
            if not audit:
                events = case._runtime.run_checked(case._runtime.viewer_command(
                    "events", "--trace-id", str(trace_id), "--output-format", "json"
                ), echo=False)
                recorded = json.loads(events.stdout)["events"]
                if not any(event.get("variant") == "process" for event in recorded):
                    raise AssertionError("ordinary process events are missing from the trace")
                if any(event.get("payload", {}).get("backend") == "bpf-lsm" for event in recorded):
                    raise AssertionError("LSM enforcement events were retained with audit disabled")
            if capacity == 1:
                log = (case._work / "log/actraild.log").read_text(encoding="utf-8")
                if log.count("LSM boundary alert aggregation capacity exceeded") != 1:
                    raise AssertionError("capacity loss was not summarized exactly once on shutdown")
            results[name] = TestResult(
                TestStatus.PASSED,
                f"trace-{trace_id}: 128 actual denials, {len(alerts)} alerts; counts, bounds and retention verified",
            )

    def _read_alerts(self, trace_id: int) -> list[dict]:
        database = self.case._work / "data/actrail.sqlite"
        with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
            rows = connection.execute(
                "SELECT d.kind, a.payload_json FROM alerts a "
                "JOIN alert_definitions d USING(alert_definition_id) "
                "WHERE a.trace_id = ? ORDER BY a.alert_id", (trace_id,),
            ).fetchall()
        return [{"kind": kind, "payload": json.loads(payload)} for kind, payload in rows]

    def _wait_for_alerts(self, trace_id: int, capacity: int) -> list[dict]:
        deadline = time.monotonic() + self.case._timeout
        while time.monotonic() < deadline:
            self.case._runtime.run_checked(self.case._runtime.control_command("list-traces"), echo=False)
            alerts = self._read_alerts(trace_id)
            totals = self._totals(alerts)
            expected = {"lsm-file-deny": 64}
            if capacity > 1:
                expected["lsm-command-deny"] = 64
            if totals == expected:
                return alerts
            time.sleep(self.case._poll)
        raise AssertionError(f"boundary alert deltas did not reach received denials: {alerts}")

    @staticmethod
    def _totals(alerts: list[dict]) -> dict[str, int]:
        totals = {}
        for alert in alerts:
            payload = alert["payload"]
            rule = payload["rule_id"]
            totals[rule] = totals.get(rule, 0) + payload["aggregation"]["received_denials"]
        return totals

    def _require_alerts(self, alerts: list[dict], capacity: int, *, periodic: bool) -> None:
        for domain in ("file", "command") if capacity > 1 else ("file",):
            group = [alert for alert in alerts if alert["payload"]["rule_id"] == f"lsm-{domain}-deny"]
            counts = [alert["payload"]["aggregation"]["received_denials"] for alert in group]
            if counts[0] != 1 or sum(counts) != 64:
                raise AssertionError(f"first alert or received deltas are wrong: {group}")
            if periodic:
                if not 2 < len(group) < 32:
                    raise AssertionError(f"timer did not summarize ongoing denials: {group}")
            elif counts != [1, 63]:
                raise AssertionError(f"trace finalization did not flush exactly one tail: {group}")
            for alert in group:
                payload = alert["payload"]
                aggregation = payload["aggregation"]
                expected_kind = "file.access.boundary-violation" if domain == "file" else "command.execution.boundary-violation"
                if alert["kind"] != expected_kind or aggregation["backend"] != "bpf-lsm":
                    raise AssertionError(f"wrong boundary alert routing: {alert}")
                expected_revision = 0 if domain == "file" else 1
                if payload["policy_owner_instance_id"] != "actrail.static" or aggregation["rule_revision"] != expected_revision:
                    raise AssertionError(f"wrong policy provenance: {alert}")
                if "matched_rule_path" in payload or payload["process_id"] <= 0:
                    raise AssertionError(f"invented rule path or missing process identity: {alert}")
                first, last = aggregation["first_observed_at"], aggregation["last_observed_at"]
                if (first["secs_since_epoch"], first["nanos_since_epoch"]) > (last["secs_since_epoch"], last["nanos_since_epoch"]):
                    raise AssertionError(f"invalid observation interval: {alert}")
