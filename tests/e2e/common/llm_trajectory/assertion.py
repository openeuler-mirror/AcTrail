from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from .environment import TrajectoryTestEnvironment


class ScenarioPreconditionFailure(AssertionError):
    pass


class ProductAssertionFailure(AssertionError):
    pass


@dataclass(frozen=True)
class LlmCallPair:
    request: dict[str, Any]
    response: dict[str, Any]


class TrajectoryAssertionSupport:
    _TRAJECTORY_ID = "llm.request.trajectory_id"

    def __init__(
        self,
        environment: TrajectoryTestEnvironment,
        trace_id: int,
    ):
        self._environment = environment
        self._trace_id = trace_id

    def _call_pairs(
        self,
        document: dict[str, Any],
        actions: list[dict[str, Any]],
    ) -> list[LlmCallPair]:
        by_id = {
            action_id: action
            for action in actions
            if (action_id := action.get("action_id"))
            and isinstance(action_id, str)
        }
        calls = {
            action_id
            for action_id, action in by_id.items()
            if action.get("kind") == "llm.call"
        }
        requests = {
            action_id
            for action_id, action in by_id.items()
            if action.get("kind") == "llm.request"
        }
        responses = {
            action_id
            for action_id, action in by_id.items()
            if action.get("kind") == "llm.response"
        }
        if not calls or len(calls) != len(requests) or len(requests) != len(responses):
            raise ScenarioPreconditionFailure(
                "LLM call/request/response count mismatch: "
                f"{len(calls)} call(s), {len(requests)} request(s), "
                f"{len(responses)} response(s)"
            )
        requests_by_call: dict[str, set[str]] = {}
        responses_by_call: dict[str, set[str]] = {}
        for link in self._object_list(document, "links"):
            if link.get("valid") is not True:
                continue
            call_id = link.get("parent_action_id")
            child_id = link.get("child_action_id")
            if not isinstance(call_id, str) or not isinstance(child_id, str):
                continue
            role = link.get("role")
            if role == "llm.call.request":
                requests_by_call.setdefault(call_id, set()).add(child_id)
            elif role == "llm.call.response":
                responses_by_call.setdefault(call_id, set()).add(child_id)

        pairs: list[LlmCallPair] = []
        paired_requests: set[str] = set()
        paired_responses: set[str] = set()
        for call_id in sorted(calls):
            request_ids = requests_by_call.get(call_id, set())
            response_ids = responses_by_call.get(call_id, set())
            if len(request_ids) != 1 or len(response_ids) != 1:
                raise ScenarioPreconditionFailure(
                    f"LLM call {call_id} does not have exactly one valid "
                    "request link and one valid response link"
                )
            request_id = next(iter(request_ids))
            response_id = next(iter(response_ids))
            if request_id not in requests or response_id not in responses:
                raise ScenarioPreconditionFailure(
                    f"LLM call {call_id} links to an action with the wrong kind"
                )
            if request_id in paired_requests or response_id in paired_responses:
                raise ScenarioPreconditionFailure(
                    f"LLM call {call_id} reuses request or response action"
                )
            paired_requests.add(request_id)
            paired_responses.add(response_id)
            pairs.append(LlmCallPair(by_id[request_id], by_id[response_id]))
        if paired_requests != requests or paired_responses != responses:
            raise ScenarioPreconditionFailure(
                "unpaired LLM actions remain after validating call links: "
                f"requests={sorted(requests - paired_requests)}, "
                f"responses={sorted(responses - paired_responses)}"
            )
        return pairs

    @staticmethod
    def _action_id(action: dict[str, Any]) -> str:
        action_id = action.get("action_id")
        if not isinstance(action_id, str) or not action_id:
            raise ScenarioPreconditionFailure("LLM action has no action_id")
        return action_id

    @staticmethod
    def _object_list(document: dict[str, Any], key: str) -> list[dict[str, Any]]:
        value = document.get(key)
        if not isinstance(value, list):
            raise ScenarioPreconditionFailure(f"document has no {key} array")
        return [item for item in value if isinstance(item, dict)]

    @staticmethod
    def _otel_attribute(container: Any, key: str) -> str | None:
        if not isinstance(container, dict):
            return None
        attributes = container.get("attributes")
        if not isinstance(attributes, list):
            return None
        for attribute in attributes:
            if not isinstance(attribute, dict) or attribute.get("key") != key:
                continue
            value = attribute.get("value")
            if not isinstance(value, dict):
                return None
            string_value = value.get("stringValue")
            if isinstance(string_value, str):
                return string_value
            int_value = value.get("intValue")
            if isinstance(int_value, (str, int)):
                return str(int_value)
        return None
