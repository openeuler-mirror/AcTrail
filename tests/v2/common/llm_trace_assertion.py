from __future__ import annotations

import json
import re
import time
from pathlib import Path

from .actrail_runtime import ActrailRuntime, CommandResult


class LLMTraceAssertion:
    _TRACE_PATTERN = re.compile(r"trace trace-(\d+) entered Active")

    def __init__(
        self,
        runtime: ActrailRuntime,
        marker: str,
        drain_attempts: int,
        drain_interval_seconds: float,
    ):
        self._runtime = runtime
        self._marker = marker
        self._drain_attempts = drain_attempts
        self._drain_interval_seconds = drain_interval_seconds

    def require_trace_id(
        self,
        launch: CommandResult,
        *,
        expected_count: int,
        selected_index: int,
    ) -> int:
        trace_ids = [
            int(value) for value in self._TRACE_PATTERN.findall(launch.output)
        ]
        if len(trace_ids) != expected_count:
            raise AssertionError(
                f"launch must report {expected_count} trace id(s); found {trace_ids}"
            )
        if len(set(trace_ids)) != len(trace_ids):
            raise AssertionError(f"launch trace ids must be distinct: {trace_ids}")
        try:
            return trace_ids[selected_index]
        except IndexError as error:
            raise AssertionError(
                f"selected trace index {selected_index} is absent from {trace_ids}"
            ) from error

    def require_answer_marker(
        self,
        launch: CommandResult,
        agent_name: str,
    ) -> None:
        if self._marker not in launch.stdout:
            raise AssertionError(
                f"{agent_name} stdout answer does not contain marker {self._marker}"
            )

    def wait_and_require_exchange(self, trace_id: int) -> tuple[int, int]:
        last_actions = ""
        last_trace_state = "missing"
        for _ in range(self._drain_attempts):
            traces = self._read_json(
                [
                    self._runtime.actrailviewer,
                    "--output-format",
                    "json",
                    "traces",
                ]
            )
            ready, last_trace_state = self._trace_is_cleanly_exited(traces, trace_id)
            if not ready:
                time.sleep(self._drain_interval_seconds)
                continue
            last_actions = self._runtime.run_checked(
                [
                    self._runtime.actrailviewer,
                    "--output-format",
                    "json",
                    "actions",
                    "--trace-id",
                    str(trace_id),
                ],
                echo=False,
            ).stdout
            document = json.loads(last_actions)
            if self._has_complete_exchange(document):
                return self._require_exchange(document)
            time.sleep(self._drain_interval_seconds)
        raise AssertionError(
            f"trace-{trace_id} did not produce a complete LLM exchange; "
            f"trace_state={last_trace_state} last_actions={last_actions}"
        )

    def _read_json(self, command: list[Path | str]) -> dict:
        output = self._runtime.run_checked(command, echo=False).stdout
        document = json.loads(output)
        if not isinstance(document, dict):
            raise AssertionError("viewer JSON output must be an object")
        return document

    def _trace_is_cleanly_exited(
        self,
        document: dict,
        trace_id: int,
    ) -> tuple[bool, str]:
        trace = next(
            (
                row
                for row in document.get("traces", [])
                if row.get("trace_id_raw") == trace_id
            ),
            None,
        )
        if trace is None:
            return False, "missing"
        state = f"state={trace.get('state')} health={trace.get('health')}"
        return trace.get("state") == "Exited" and trace.get("health") == "Clean", state

    def _has_complete_exchange(self, document: dict) -> bool:
        kinds = {
            action.get("kind")
            for action in self._complete_actions(document)
        }
        return {"llm.call", "llm.request", "llm.response"} <= kinds

    def _require_exchange(self, document: dict) -> tuple[int, int]:
        complete = self._complete_actions(document)
        requests = [
            action for action in complete if action.get("kind") == "llm.request"
        ]
        responses = [
            action for action in complete if action.get("kind") == "llm.response"
        ]
        if not requests or not responses:
            raise AssertionError("trace has no complete LLM request/response")
        if len(requests) != len(responses):
            raise AssertionError(
                "LLM request/response count mismatch: "
                f"{len(requests)} request(s), {len(responses)} response(s)"
            )
        self._require_marker(requests, "request")
        self._require_marker(responses, "response")
        self._require_one_to_one_call_links(document, requests, responses)
        return len(requests), len(responses)

    def _complete_actions(self, document: dict) -> list[dict]:
        return [
            action
            for action in document.get("actions", [])
            if action.get("completeness") == "complete"
        ]

    def _require_marker(self, actions: list[dict], side: str) -> None:
        if any(
            self._marker in json.dumps(action, ensure_ascii=False)
            for action in actions
        ):
            return
        raise AssertionError(f"captured LLM {side} does not contain {self._marker}")

    def _require_one_to_one_call_links(
        self,
        document: dict,
        requests: list[dict],
        responses: list[dict],
    ) -> None:
        request_ids = {action["action_id"] for action in requests}
        response_ids = {action["action_id"] for action in responses}
        request_links: dict[str, set[str]] = {}
        response_links: dict[str, set[str]] = {}
        for link in document.get("links", []):
            call_id = link.get("parent_action_id")
            child_id = link.get("child_action_id")
            if link.get("role") == "llm.call.request" and child_id in request_ids:
                request_links.setdefault(call_id, set()).add(child_id)
            if link.get("role") == "llm.call.response" and child_id in response_ids:
                response_links.setdefault(call_id, set()).add(child_id)
        paired_requests: set[str] = set()
        paired_responses: set[str] = set()
        for call_id in request_links.keys() & response_links.keys():
            call_requests = request_links[call_id]
            call_responses = response_links[call_id]
            if len(call_requests) != 1 or len(call_responses) != 1:
                raise AssertionError(
                    f"LLM call {call_id} is not a one-request/one-response exchange"
                )
            paired_requests.update(call_requests)
            paired_responses.update(call_responses)
        if paired_requests != request_ids or paired_responses != response_ids:
            raise AssertionError(
                "not every complete LLM request/response is paired by one llm.call"
            )
