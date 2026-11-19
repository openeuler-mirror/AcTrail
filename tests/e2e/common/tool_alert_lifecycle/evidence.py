from __future__ import annotations

import json
from pathlib import Path
import shlex
from typing import Any

from tests.e2e.common.core import TestResult, TestStatus


class CommandTerminalEvidence:
    """Check real exec replacements against persisted alert evidence."""

    def __init__(self, environment: Any):
        self._environment = environment

    def verify(
        self, trace_id: int, payload: dict[str, Any], paths: tuple[str, ...]
    ) -> TestResult:
        actions = self._environment.viewer_actions(trace_id)
        commands = [action for action in actions if action.get("kind") == "command.invocation"]
        ls_commands = [action for action in commands if self._executable(action) == "ls"]
        failed = [
            action for action in ls_commands
            if set(shlex.split(action["attributes"].get("command.line", ""))) & set(paths)
        ]
        if len(failed) != len(paths):
            raise AssertionError(f"trace-{trace_id}: expected {len(paths)} real ls commands, got {len(failed)}")
        observed_paths = {
            token for action in failed
            for token in shlex.split(action["attributes"]["command.line"])
            if token in paths
        }
        if observed_paths != set(paths):
            raise AssertionError(f"trace-{trace_id}: captured ls paths differ: {observed_paths}")

        superseded: set[str] = set()
        for action in failed:
            if (action["status"], action["completeness"], action["attributes"].get("command.exit_code")) != ("error", "complete", "2"):
                raise AssertionError(f"ls has no observed failing terminal: {action}")
            start = int(action["start_time_unix_nanos"])
            end = int(action["end_time_unix_nanos"])
            if end < start:
                raise AssertionError(f"negative command duration: {action['action_id']}")
            previous = [
                command for command in commands
                if self._executable(command) == "bash"
                and command["process"]["process_id"] == action["process"]["process_id"]
                and int(command["start_time_unix_nanos"]) <= start
                and command.get("end_time_unix_nanos") is not None
                and int(command["end_time_unix_nanos"]) <= start
                and (command["status"], command["completeness"]) == ("unknown", "complete")
            ]
            if not previous:
                raise AssertionError(f"no same-process bash terminal before ls: {action['action_id']}")
            superseded.update(command["action_id"] for command in previous)

        evidence = set(payload["evidence_action_ids"])
        # Process exits are transient actions. Command IDs are durable and use
        # the producer's trace:event:kind identity, as in the existing checks.
        command_evidence = {
            action_id for action_id in evidence
            if action_id.endswith(":command.invocation")
        }
        expected = {action["action_id"] for action in failed}
        allowed = {action["action_id"] for action in ls_commands}
        if not expected.issubset(command_evidence) or not command_evidence.issubset(allowed):
            raise AssertionError(f"alert command evidence differs from real ls: expected={sorted(expected)}, actual={sorted(command_evidence)}")
        if command_evidence & superseded:
            raise AssertionError("superseded bash terminal changed command registration")
        return TestResult(TestStatus.PASSED, json.dumps({
            "trace_id": trace_id,
            "failed_ls_ids": sorted(expected),
            "superseded_bash_terminal_ids": sorted(superseded),
            "alert_command_evidence_ids": sorted(command_evidence),
            "failure_count": payload.get("failure_count", payload.get("consecutive_failures")),
            "total_count": payload.get("total_count"),
        }, sort_keys=True))

    @staticmethod
    def _executable(action: dict[str, Any]) -> str:
        return Path(action["attributes"].get("process.executable", "")).name
