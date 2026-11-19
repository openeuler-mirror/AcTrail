from __future__ import annotations

import json
import time


class CommandAuditVerification:
    def __init__(self, environment):
        self._environment = environment
        self._config = environment.command_config

    def verify(self, trace_id: int) -> None:
        deadline = time.monotonic() + self._config.evidence_timeout_seconds
        while time.monotonic() < deadline:
            traces = self._read("traces")["traces"]
            if any(
                trace.get("trace_id_raw") == trace_id
                and trace.get("state") in {"Exited", "Completed"}
                and trace.get("health") == "Clean"
                for trace in traces
            ):
                break
            time.sleep(self._config.drain_interval_seconds)
        else:
            raise AssertionError(f"trace-{trace_id} did not reach a clean terminal state")
        events = self._read("events", "--trace-id", str(trace_id))["events"]
        actions = self._read("actions", "--trace-id", str(trace_id))["actions"]
        # Audit settings cover ordinary decisions and native target lookup.
        sources = {"target-lookup", "rule", "default", "gray-plugin", "gray-plugin-cache", "gray-fallback"}
        audited_events = [
            event for event in events
            if event.get("variant") == "enforcement"
            and event["payload"].get("backend") == "seccomp-user-notify"
            and event["payload"].get("metadata", {}).get("decision_source") in sources
        ]
        audited_actions = [
            action for action in actions
            if action.get("kind") == "enforcement.decision"
            and action.get("attributes", {}).get("decision_source") in sources
        ]
        if self._config.audit_enabled:
            lookup_events = [event for event in audited_events if event["payload"]["metadata"]["decision_source"] == "target-lookup"]
            lookup_actions = [action for action in audited_actions if action["attributes"]["decision_source"] == "target-lookup"]
            if len(lookup_events) < 2 or len(lookup_actions) != len(lookup_events):
                raise AssertionError(f"trace-{trace_id} native lookup event/action delivery is incomplete")
        elif audited_events or audited_actions:
            raise AssertionError(
                f"audit disabled but trace-{trace_id} persisted {len(audited_events)} events and {len(audited_actions)} actions"
            )

    def _read(self, *arguments):
        result = self._environment.runtime.run_checked(
            self._environment.runtime.viewer_command(*arguments, "--output-format", "json"),
            echo=False,
        )
        return json.loads(result.stdout)
