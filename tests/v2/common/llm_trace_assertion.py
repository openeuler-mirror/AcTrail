from __future__ import annotations

import json
import re
from collections import Counter
from pathlib import Path

from .actrail_runtime import ActrailRuntime, CommandResult


class LLMTraceAssertion:
    _TRACE_PATTERN = re.compile(r"trace trace-(\d+) entered Active")
    _LLM_ACTION_KINDS = frozenset(
        {"llm.call", "llm.request", "llm.response", "sse.stream"}
    )
    _TRACE_CLOSE_ATTRIBUTE = "actrail.action.finalized_on_trace_close"
    _RESPONSE_OUTPUT_ATTRIBUTES = (
        "llm.response.content_text",
        "llm.response.output_text",
    )
    _REQUEST_CONTENT_STATE_ATTRIBUTE = "llm.request.content_state"
    _REQUEST_CONTENT_HASH_ATTRIBUTE = "llm.request.canonical_body_hash"
    _REQUEST_CONTENT_BYTES_ATTRIBUTE = "llm.request.canonical_body_bytes"

    def __init__(
        self,
        runtime: ActrailRuntime,
        marker: str,
    ):
        self._runtime = runtime
        self._marker = marker

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

    def require_finalized_exchange(self, trace_id: int) -> tuple[int, int]:
        traces = self._read_json(
            self._runtime.viewer_command(
                "--output-format",
                "json",
                "traces",
            )
        )
        ready, trace_state = self._trace_is_cleanly_terminal(traces, trace_id)
        if not ready:
            raise AssertionError(
                f"trace-{trace_id} must be cleanly terminal after daemon shutdown; "
                f"{trace_state}"
            )
        document = self._read_json(
            self._runtime.viewer_command(
                "--output-format",
                "json",
                "actions",
                "--trace-id",
                str(trace_id),
            )
        )
        actions = self._llm_actions(document)
        self._require_terminal_actions(actions)
        return self._require_paired_exchange(document, actions)

    def _read_json(self, command: list[Path | str]) -> dict:
        output = self._runtime.run_checked(command, echo=False).stdout
        document = json.loads(output)
        if not isinstance(document, dict):
            raise AssertionError("viewer JSON output must be an object")
        return document

    def _trace_is_cleanly_terminal(
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
        return (
            trace.get("state") in {"Completed", "Exited"}
            and trace.get("health") == "Clean"
        ), state

    def _require_paired_exchange(
        self,
        document: dict,
        actions: list[dict],
    ) -> tuple[int, int]:
        calls = [action for action in actions if action.get("kind") == "llm.call"]
        requests = [action for action in actions if action.get("kind") == "llm.request"]
        responses = [action for action in actions if action.get("kind") == "llm.response"]
        if not requests or not responses:
            raise AssertionError("trace has no LLM request/response")
        if len(calls) != len(requests):
            raise AssertionError(
                "LLM call/request count mismatch: "
                f"{len(calls)} call(s), {len(requests)} request(s), "
                f"{len(responses)} response(s)"
            )
        pairs = self._require_one_to_one_call_links(
            document,
            calls,
            requests,
            responses,
        )
        self._require_marker_exchange(pairs)
        return len(requests), len(responses)

    def _llm_actions(self, document: dict) -> list[dict]:
        return [
            action
            for action in document.get("actions", [])
            if action.get("kind") in self._LLM_ACTION_KINDS
        ]

    def _require_terminal_actions(self, actions: list[dict]) -> None:
        if not actions:
            raise AssertionError("trace has no LLM actions")
        for action in actions:
            kind = action.get("kind")
            status = action.get("status")
            completeness = action.get("completeness")
            if status == "in_progress":
                raise AssertionError(
                    f"{action.get('action_id')} ({kind}) is still in_progress "
                    "after daemon shutdown"
                )
            if completeness == "complete":
                continue
            finalized_on_close = (
                action.get("attributes", {}).get(self._TRACE_CLOSE_ATTRIBUTE)
                == "true"
            )
            if (
                kind in {"llm.call", "llm.response", "sse.stream"}
                and status == "error"
                and completeness == "partial"
                and finalized_on_close
            ):
                continue
            raise AssertionError(
                f"{action.get('action_id')} ({kind}) has invalid terminal state "
                f"status={status} completeness={completeness} "
                f"finalized_on_trace_close={finalized_on_close}"
            )

    def _require_one_to_one_call_links(
        self,
        document: dict,
        calls: list[dict],
        requests: list[dict],
        responses: list[dict],
    ) -> list[tuple[dict, dict]]:
        calls_by_id = {action["action_id"]: action for action in calls}
        requests_by_id = {action["action_id"]: action for action in requests}
        responses_by_id = {action["action_id"]: action for action in responses}
        request_links: dict[str, set[str]] = {}
        response_links: dict[str, set[str]] = {}
        failed_http_requests = self._failed_http_request_counts(document)
        for link in document.get("links", []):
            if not link.get("valid", False):
                continue
            call_id = link.get("parent_action_id")
            child_id = link.get("child_action_id")
            if (
                call_id in calls_by_id
                and link.get("role") == "llm.call.request"
                and child_id in requests_by_id
            ):
                request_links.setdefault(call_id, set()).add(child_id)
            if (
                call_id in calls_by_id
                and link.get("role") == "llm.call.response"
                and child_id in responses_by_id
            ):
                response_links.setdefault(call_id, set()).add(child_id)
        paired_requests: set[str] = set()
        paired_responses: set[str] = set()
        pairs: list[tuple[dict, dict]] = []
        for call_id in calls_by_id:
            call_requests = request_links.get(call_id, set())
            call_responses = response_links.get(call_id, set())
            if len(call_requests) != 1 or len(call_responses) > 1:
                raise AssertionError(
                    f"LLM call {call_id} does not have exactly one request "
                    "and at most one response"
                )
            paired_requests.update(call_requests)
            request_id = next(iter(call_requests))
            request = requests_by_id[request_id]
            if call_responses:
                paired_responses.update(call_responses)
                response_id = next(iter(call_responses))
                pairs.append((request, responses_by_id[response_id]))
                continue
            self._require_failed_http_probe(
                calls_by_id[call_id],
                request,
                failed_http_requests,
            )
        if (
            paired_requests != set(requests_by_id)
            or paired_responses != set(responses_by_id)
        ):
            raise AssertionError(
                "not every LLM request/response is paired by exactly one llm.call"
            )
        return pairs

    def _failed_http_request_counts(
        self,
        document: dict,
    ) -> Counter[tuple[str, str, str]]:
        http_requests: dict[str, tuple[str, str, str]] = {}
        http_responses: list[dict] = []
        for action in document.get("actions", []):
            if action.get("kind") != "http.message":
                continue
            attributes = action.get("attributes")
            if not isinstance(attributes, dict):
                continue
            operation = attributes.get("http.operation")
            if operation == "request":
                key = self._http_request_key(
                    attributes,
                    stream_key_name="stream_key",
                    method_name="method",
                    path_name="target",
                )
                action_id = action.get("action_id")
                if key is not None and isinstance(action_id, str):
                    http_requests[action_id] = key
            elif operation == "response":
                http_responses.append(attributes)

        failed: Counter[tuple[str, str, str]] = Counter()
        for response in http_responses:
            try:
                status_code = int(response.get("status_code", ""))
            except (TypeError, ValueError):
                continue
            if status_code < 400:
                continue
            request_id = response.get("http.request.action_id")
            if isinstance(request_id, str) and request_id in http_requests:
                failed[http_requests[request_id]] += 1
        return failed

    def _require_failed_http_probe(
        self,
        call: dict,
        request: dict,
        failed_http_requests: Counter[tuple[str, str, str]],
    ) -> None:
        attributes = call.get("attributes")
        if not isinstance(attributes, dict):
            attributes = {}
        terminal_partial = (
            call.get("status") == "error"
            and call.get("completeness") == "partial"
            and attributes.get(self._TRACE_CLOSE_ATTRIBUTE) == "true"
        )
        request_attributes = request.get("attributes")
        key = (
            self._http_request_key(
                request_attributes,
                stream_key_name="payload.stream_key",
                method_name="http.request.method",
                path_name="url.path",
            )
            if isinstance(request_attributes, dict)
            else None
        )
        if not terminal_partial or key is None or failed_http_requests[key] == 0:
            raise AssertionError(
                f"LLM call {call.get('action_id')} has no response and no "
                "correlated failed HTTP probe"
            )
        failed_http_requests[key] -= 1

    @staticmethod
    def _http_request_key(
        attributes: dict,
        *,
        stream_key_name: str,
        method_name: str,
        path_name: str,
    ) -> tuple[str, str, str] | None:
        values = (
            attributes.get(stream_key_name),
            attributes.get(method_name),
            attributes.get(path_name),
        )
        if not all(isinstance(value, str) and value for value in values):
            return None
        stream_key, method, path = values
        return stream_key, method.upper(), path

    def _require_marker_exchange(self, pairs: list[tuple[dict, dict]]) -> None:
        for request, response in pairs:
            request_attributes = request.get("attributes", {})
            request_has_canonical_content = (
                request_attributes.get(self._REQUEST_CONTENT_STATE_ATTRIBUTE)
                == "canonical_blocks"
                and request_attributes.get(
                    self._REQUEST_CONTENT_HASH_ATTRIBUTE,
                    "",
                ).startswith("sha256:")
                and int(
                    request_attributes.get(
                        self._REQUEST_CONTENT_BYTES_ATTRIBUTE,
                        "0",
                    )
                )
                > 0
            )
            response_attributes = response.get("attributes", {})
            response_contains_marker = any(
                self._marker in response_attributes.get(key, "")
                for key in self._RESPONSE_OUTPUT_ATTRIBUTES
            )
            if request_has_canonical_content and response_contains_marker:
                return
        raise AssertionError(
            "no LLM call links a canonical-content request to a normalized "
            f"response containing {self._marker}"
        )
