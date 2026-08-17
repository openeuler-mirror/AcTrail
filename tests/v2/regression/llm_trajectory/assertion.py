from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

from .environment import LlmTrajectoryEnvironment


class ScenarioPreconditionFailure(AssertionError):
    pass


class ProductAssertionFailure(AssertionError):
    pass


@dataclass(frozen=True)
class LlmCallPair:
    request: dict[str, Any]
    response: dict[str, Any]
    request_body: dict[str, Any]


@dataclass(frozen=True)
class TrajectoryEvidence:
    title: LlmCallPair
    main_first: LlmCallPair
    main_second: LlmCallPair
    subagent_first: LlmCallPair
    subagent_second: LlmCallPair

    @property
    def request_pairs(self) -> tuple[tuple[str, LlmCallPair], ...]:
        return (
            ("title", self.title),
            ("main_first", self.main_first),
            ("main_second", self.main_second),
            ("subagent_first", self.subagent_first),
            ("subagent_second", self.subagent_second),
        )


class LlmTrajectoryAssertion:
    _TRAJECTORY_ID = "llm.request.trajectory_id"
    _INFERENCE_VERSION = "llm.request.trajectory_inference_version"
    _TOOL_CALLS = "llm.response.tool_calls_json"
    _COMMAND_LINE = "command.line"
    _COMMAND_EXIT_CODE = "command.exit_code"
    _BACKGROUND_KIND = "llm.request.background_kind"
    _SHELL_TOOL_NAMES = frozenset({"bash", "shell", "exec", "terminal"})

    def __init__(
        self,
        environment: LlmTrajectoryEnvironment,
        trace_id: int,
        task_marker: str,
        expected_commit: str,
        request_content_max_bytes: int,
    ):
        self._environment = environment
        self._trace_id = trace_id
        self._task_marker = task_marker
        self._expected_commit = expected_commit
        self._request_content_max_bytes = request_content_max_bytes

    def require_scenario(self, document: dict[str, Any]) -> TrajectoryEvidence:
        actions = self._object_list(document, "actions")
        pairs = self._call_pairs(document, actions)
        if len(pairs) < 5:
            raise ScenarioPreconditionFailure(
                f"expected at least five paired LLM calls, found {len(pairs)}"
            )
        enriched = [
            LlmCallPair(pair.request, pair.response, self._request_body(pair.request))
            for pair in pairs
        ]
        main_first = self._unique_pair(
            enriched,
            lambda pair: "task" in self._tool_names(pair.response),
            "main response that executed the task/subagent tool",
        )
        subagent_first = self._unique_pair(
            enriched,
            lambda pair: (
                pair.request["action_id"] != main_first.request["action_id"]
                and self._task_marker in self._compact_json(pair.request_body)
                and bool(self._tool_names(pair.response) & self._SHELL_TOOL_NAMES)
            ),
            "subagent request whose response executed a shell tool",
        )
        subagent_second = self._direct_history_child(
            subagent_first,
            enriched,
            "subagent tool-result continuation",
        )
        main_second = self._direct_history_child(
            main_first,
            enriched,
            "main continuation after the subagent result",
        )
        title = self._unique_pair(
            enriched,
            lambda pair: (
                pair.request["action_id"]
                not in {
                    main_first.request["action_id"],
                    main_second.request["action_id"],
                    subagent_first.request["action_id"],
                    subagent_second.request["action_id"],
                }
                and self._is_title_request(pair.request)
            ),
            "OpenCode title-generation request",
        )
        self._require_git_execution(actions)
        return TrajectoryEvidence(
            title=title,
            main_first=main_first,
            main_second=main_second,
            subagent_first=subagent_first,
            subagent_second=subagent_second,
        )

    def require_trajectory(self, evidence: TrajectoryEvidence) -> dict[str, str]:
        lineages: dict[str, dict[str, Any]] = {}
        trajectory_ids: dict[str, str] = {}
        for role, pair in evidence.request_pairs:
            action_id = self._action_id(pair.request)
            document = self._environment.api.llm_request_lineage(
                self._trace_id,
                action_id,
            )
            lineage = document.get("lineage")
            if not isinstance(lineage, dict):
                raise ProductAssertionFailure(
                    f"{role} request {action_id} has no persisted lineage"
                )
            trajectory_id = lineage.get("trajectory_id")
            if not isinstance(trajectory_id, str) or not trajectory_id:
                raise ProductAssertionFailure(
                    f"{role} request {action_id} has no trajectory id"
                )
            action_attributes = pair.request.get("attributes")
            if not isinstance(action_attributes, dict):
                raise ProductAssertionFailure(f"{role} request has no attributes")
            if action_attributes.get(self._TRAJECTORY_ID) != trajectory_id:
                raise ProductAssertionFailure(
                    f"{role} action and lineage trajectory ids differ: "
                    f"{action_attributes.get(self._TRAJECTORY_ID)!r} != {trajectory_id!r}"
                )
            if self._INFERENCE_VERSION not in action_attributes:
                raise ProductAssertionFailure(
                    f"{role} request has no trajectory inference version"
                )
            lineages[role] = lineage
            trajectory_ids[role] = trajectory_id

        title_id = trajectory_ids["title"]
        main_id = trajectory_ids["main_first"]
        subagent_id = trajectory_ids["subagent_first"]
        if len({title_id, main_id, subagent_id}) != 3:
            raise ProductAssertionFailure(
                "title, main, and subagent trajectories are not distinct: "
                f"title={title_id}, main={main_id}, subagent={subagent_id}"
            )
        self._require_continuation(
            "main",
            evidence.main_first,
            evidence.main_second,
            lineages["main_first"],
            lineages["main_second"],
        )
        self._require_continuation(
            "subagent",
            evidence.subagent_first,
            evidence.subagent_second,
            lineages["subagent_first"],
            lineages["subagent_second"],
        )
        if lineages["title"].get("parent_action_id") is not None:
            raise ProductAssertionFailure("title request unexpectedly has a parent")
        title_trajectory = self._environment.api.llm_request_trajectory(
            self._trace_id,
            title_id,
        )
        title_nodes = title_trajectory.get("nodes")
        if not isinstance(title_nodes, list) or len(title_nodes) != 1:
            raise ProductAssertionFailure(
                "title trajectory must have exactly one node; "
                f"observed={title_nodes!r}"
            )
        return trajectory_ids

    def require_otel(
        self,
        evidence: TrajectoryEvidence,
        trajectory_ids: dict[str, str],
        spans: list[dict[str, Any]],
    ) -> None:
        spans_by_action: dict[str, list[dict[str, Any]]] = {}
        for span in spans:
            action_id = self._otel_attribute(span, "actrail.action.id")
            if action_id:
                spans_by_action.setdefault(action_id, []).append(span)
        for role, pair in evidence.request_pairs:
            action_id = self._action_id(pair.request)
            matches = spans_by_action.get(action_id, [])
            if len(matches) != 1:
                raise ProductAssertionFailure(
                    f"OTel expected one {role} request span for {action_id}, "
                    f"found {len(matches)}"
                )
            span = matches[0]
            if self._otel_attribute(span, "actrail.action.kind") != "llm.request":
                raise ProductAssertionFailure(f"OTel {role} span is not llm.request")
            if self._otel_attribute(span, self._TRAJECTORY_ID) != trajectory_ids[role]:
                raise ProductAssertionFailure(
                    f"OTel {role} trajectory id differs from persisted lineage"
                )
            if self._otel_attribute(span, self._INFERENCE_VERSION) is None:
                raise ProductAssertionFailure(
                    f"OTel {role} span has no trajectory inference version"
                )

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
        requests_by_call: dict[str, str] = {}
        responses_by_call: dict[str, str] = {}
        for link in self._object_list(document, "links"):
            if link.get("valid") is not True:
                continue
            call_id = link.get("parent_action_id")
            child_id = link.get("child_action_id")
            if not isinstance(call_id, str) or not isinstance(child_id, str):
                continue
            role = link.get("role")
            if role == "llm.call.request":
                requests_by_call[call_id] = child_id
            elif role == "llm.call.response":
                responses_by_call[call_id] = child_id
        pairs: list[LlmCallPair] = []
        for call_id in sorted(requests_by_call.keys() & responses_by_call.keys()):
            request = by_id.get(requests_by_call[call_id])
            response = by_id.get(responses_by_call[call_id])
            if (
                isinstance(request, dict)
                and request.get("kind") == "llm.request"
                and isinstance(response, dict)
                and response.get("kind") == "llm.response"
            ):
                pairs.append(LlmCallPair(request, response, {}))
        return pairs

    def _request_body(self, request: dict[str, Any]) -> dict[str, Any]:
        action_id = self._action_id(request)
        document = self._environment.api.llm_request_content(
            self._trace_id,
            action_id,
            max_bytes=self._request_content_max_bytes,
        )
        content = document.get("content")
        if not isinstance(content, dict):
            raise ScenarioPreconditionFailure(
                f"request {action_id} has no reconstructable canonical content"
            )
        if content.get("truncated") is True:
            raise ScenarioPreconditionFailure(
                f"request {action_id} canonical content was truncated"
            )
        raw_body = content.get("body_json")
        if not isinstance(raw_body, str):
            raise ScenarioPreconditionFailure(
                f"request {action_id} content has no body_json"
            )
        try:
            body = json.loads(raw_body)
        except json.JSONDecodeError as error:
            raise ScenarioPreconditionFailure(
                f"request {action_id} body_json is invalid JSON"
            ) from error
        if not isinstance(body, dict):
            raise ScenarioPreconditionFailure(
                f"request {action_id} body_json is not an object"
            )
        return body

    def _direct_history_child(
        self,
        parent: LlmCallPair,
        pairs: list[LlmCallPair],
        description: str,
    ) -> LlmCallPair:
        parent_history = self._history(parent.request_body)
        candidates = [
            pair
            for pair in pairs
            if pair.request["action_id"] != parent.request["action_id"]
            and self._expected_commit in self._compact_json(pair.request_body)
            and self._strict_prefix(parent_history, self._history(pair.request_body))
        ]
        if not candidates:
            raise ScenarioPreconditionFailure(f"missing {description}")
        shortest = min(len(self._history(pair.request_body)) for pair in candidates)
        nearest = [
            pair for pair in candidates if len(self._history(pair.request_body)) == shortest
        ]
        if len(nearest) != 1:
            raise ScenarioPreconditionFailure(
                f"expected one {description}, found {len(nearest)} nearest candidates"
            )
        return nearest[0]

    def _require_git_execution(self, actions: list[dict[str, Any]]) -> None:
        matching: list[dict[str, Any]] = []
        for action in actions:
            if action.get("kind") != "command.invocation":
                continue
            attributes = action.get("attributes")
            if not isinstance(attributes, dict):
                continue
            command_line = attributes.get(self._COMMAND_LINE)
            if isinstance(command_line, str) and "git rev-parse HEAD" in command_line:
                matching.append(action)
        if not matching:
            raise ScenarioPreconditionFailure(
                "subagent did not execute the required git rev-parse HEAD command"
            )
        if not any(
            action.get("status") == "success"
            or str((action.get("attributes") or {}).get(self._COMMAND_EXIT_CODE)) == "0"
            for action in matching
        ):
            raise ScenarioPreconditionFailure(
                "subagent git rev-parse HEAD command did not succeed"
            )

    def _require_continuation(
        self,
        name: str,
        parent: LlmCallPair,
        child: LlmCallPair,
        parent_lineage: dict[str, Any],
        child_lineage: dict[str, Any],
    ) -> None:
        parent_id = self._action_id(parent.request)
        if parent_lineage.get("trajectory_id") != child_lineage.get("trajectory_id"):
            raise ProductAssertionFailure(f"{name} continuation changed trajectory id")
        if child_lineage.get("parent_action_id") != parent_id:
            raise ProductAssertionFailure(
                f"{name} continuation parent is {child_lineage.get('parent_action_id')!r}, "
                f"expected {parent_id!r}"
            )
        if not self._strict_prefix(
            self._history(parent.request_body),
            self._history(child.request_body),
        ):
            raise ProductAssertionFailure(
                f"{name} persisted parent does not have a strict history prefix"
            )

    def _is_title_request(self, request: dict[str, Any]) -> bool:
        attributes = request.get("attributes")
        return isinstance(attributes, dict) and (
            attributes.get(self._BACKGROUND_KIND) == "title_generation"
        )

    def _tool_names(self, response: dict[str, Any]) -> set[str]:
        attributes = response.get("attributes")
        if not isinstance(attributes, dict):
            return set()
        raw_calls = attributes.get(self._TOOL_CALLS)
        if not isinstance(raw_calls, str):
            return set()
        try:
            calls = json.loads(raw_calls)
        except json.JSONDecodeError:
            return set()
        if not isinstance(calls, list):
            return set()
        names: set[str] = set()
        for call in calls:
            if not isinstance(call, dict):
                continue
            name = call.get("name")
            function = call.get("function")
            if not isinstance(name, str) and isinstance(function, dict):
                name = function.get("name")
            if isinstance(name, str):
                names.add(name.lower())
        return names

    @staticmethod
    def _history(body: dict[str, Any]) -> list[Any]:
        for key in ("messages", "input", "prompt"):
            value = body.get(key)
            if isinstance(value, list):
                return value
            if isinstance(value, str):
                return [value]
        return []

    @staticmethod
    def _strict_prefix(parent: list[Any], child: list[Any]) -> bool:
        return bool(parent) and len(parent) < len(child) and parent == child[: len(parent)]

    @staticmethod
    def _compact_json(value: Any) -> str:
        return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))

    @staticmethod
    def _unique_pair(
        pairs: list[LlmCallPair],
        predicate: Any,
        description: str,
    ) -> LlmCallPair:
        matches = [pair for pair in pairs if predicate(pair)]
        if len(matches) != 1:
            raise ScenarioPreconditionFailure(
                f"expected one {description}, found {len(matches)}"
            )
        return matches[0]

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
