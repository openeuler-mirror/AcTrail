from __future__ import annotations

import json
from dataclasses import dataclass
from collections.abc import Callable
from typing import Any

from tests.v2.regression.llm_trajectory.assertion import (
    LlmCallPair,
    LlmTrajectoryAssertion,
    ProductAssertionFailure,
    ScenarioPreconditionFailure,
)


@dataclass(frozen=True)
class ClaudeTrajectoryEvidence:
    main_first: LlmCallPair
    main_second: LlmCallPair
    subagent_first: LlmCallPair
    subagent_second: LlmCallPair

    @property
    def request_pairs(self) -> tuple[tuple[str, LlmCallPair], ...]:
        return (
            ("main_first", self.main_first),
            ("main_second", self.main_second),
            ("subagent_first", self.subagent_first),
            ("subagent_second", self.subagent_second),
        )


class ClaudeTrajectoryAssertion(LlmTrajectoryAssertion):
    _SUBAGENT_TOOL_NAMES = frozenset({"agent", "task"})

    def require_scenario(
        self,
        document: dict[str, Any],
    ) -> ClaudeTrajectoryEvidence:
        actions = self._object_list(document, "actions")
        pairs = self._call_pairs(document, actions)
        if len(pairs) < 4:
            raise ScenarioPreconditionFailure(
                f"expected at least four paired LLM calls, found {len(pairs)}"
            )
        enriched = [
            LlmCallPair(pair.request, pair.response, self._request_body(pair.request))
            for pair in pairs
        ]
        main_first = self._unique_pair(
            enriched,
            lambda pair: bool(
                self._tool_names(pair.response) & self._SUBAGENT_TOOL_NAMES
            ),
            "main response that executed the Agent/Task subagent tool",
        )
        if self._tool_names(main_first.response) & self._SHELL_TOOL_NAMES:
            raise ScenarioPreconditionFailure(
                "main Agent response also executed a shell tool"
            )
        subagent_first = self._shortest_history_pair(
            enriched,
            lambda pair: (
                pair.request["action_id"] != main_first.request["action_id"]
                and self._task_marker in self._compact_json(pair.request_body)
                and bool(self._tool_names(pair.response) & self._SHELL_TOOL_NAMES)
                and self._response_requests_git(pair.response)
            ),
            "delegated request whose response executed Bash",
        )
        subagent_second = self._direct_history_child(
            subagent_first,
            enriched,
            "subagent continuation after the Bash result",
        )
        main_second = self._direct_history_child(
            main_first,
            enriched,
            "main continuation after the Agent result",
        )
        self._require_git_execution(actions)
        return ClaudeTrajectoryEvidence(
            main_first=main_first,
            main_second=main_second,
            subagent_first=subagent_first,
            subagent_second=subagent_second,
        )

    def _shortest_history_pair(
        self,
        pairs: list[LlmCallPair],
        predicate: Callable[[LlmCallPair], bool],
        description: str,
    ) -> LlmCallPair:
        candidates = [pair for pair in pairs if predicate(pair)]
        if not candidates:
            raise ScenarioPreconditionFailure(f"missing {description}")
        shortest_length = min(
            len(self._history(pair.request_body)) for pair in candidates
        )
        nearest = [
            pair
            for pair in candidates
            if len(self._history(pair.request_body)) == shortest_length
        ]
        if len(nearest) != 1:
            raise ScenarioPreconditionFailure(
                f"expected one nearest {description}, found {len(nearest)}"
            )
        return min(
            nearest,
            key=lambda pair: (
                len(self._history(pair.request_body)),
                self._action_id(pair.request),
            ),
        )

    def _response_requests_git(self, response: dict[str, Any]) -> bool:
        attributes = response.get("attributes")
        if not isinstance(attributes, dict):
            return False
        raw_calls = attributes.get(self._TOOL_CALLS)
        if not isinstance(raw_calls, str):
            return False
        try:
            calls = json.loads(raw_calls)
        except json.JSONDecodeError:
            return False
        return "git rev-parse HEAD" in self._compact_json(calls)

    def require_trajectory(
        self,
        evidence: ClaudeTrajectoryEvidence,
    ) -> dict[str, str]:
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
            attributes = pair.request.get("attributes")
            if not isinstance(attributes, dict):
                raise ProductAssertionFailure(f"{role} request has no attributes")
            if attributes.get(self._TRAJECTORY_ID) != trajectory_id:
                raise ProductAssertionFailure(
                    f"{role} action and lineage trajectory ids differ"
                )
            if self._INFERENCE_VERSION not in attributes:
                raise ProductAssertionFailure(
                    f"{role} request has no trajectory inference version"
                )
            lineages[role] = lineage
            trajectory_ids[role] = trajectory_id

        main_id = trajectory_ids["main_first"]
        subagent_id = trajectory_ids["subagent_first"]
        if main_id == subagent_id:
            raise ProductAssertionFailure(
                "Claude main and subagent requests share one trajectory id"
            )
        self._require_root("main", lineages["main_first"])
        self._require_root("subagent", lineages["subagent_first"])
        self._require_descendant(
            "main",
            evidence.main_first,
            evidence.main_second,
            lineages["main_first"],
            lineages["main_second"],
        )
        self._require_descendant(
            "subagent",
            evidence.subagent_first,
            evidence.subagent_second,
            lineages["subagent_first"],
            lineages["subagent_second"],
        )
        self._require_trajectory_endpoint(
            "main",
            main_id,
            evidence.main_first,
            evidence.main_second,
        )
        self._require_trajectory_endpoint(
            "subagent",
            subagent_id,
            evidence.subagent_first,
            evidence.subagent_second,
        )
        return trajectory_ids

    @staticmethod
    def _require_root(name: str, lineage: dict[str, Any]) -> None:
        if lineage.get("parent_action_id") is not None:
            raise ProductAssertionFailure(
                f"{name} first request unexpectedly has a lineage parent"
            )

    def _require_trajectory_endpoint(
        self,
        name: str,
        trajectory_id: str,
        first: LlmCallPair,
        second: LlmCallPair,
    ) -> None:
        document = self._environment.api.llm_request_trajectory(
            self._trace_id,
            trajectory_id,
        )
        nodes = document.get("nodes")
        if not isinstance(nodes, list):
            raise ProductAssertionFailure(
                f"{name} trajectory endpoint returned no nodes array"
            )
        if not all(isinstance(node, dict) for node in nodes):
            raise ProductAssertionFailure(
                f"{name} trajectory endpoint contains a non-object node"
            )
        observed = [node.get("action_id") for node in nodes]
        if not all(isinstance(action_id, str) for action_id in observed):
            raise ProductAssertionFailure(
                f"{name} trajectory endpoint contains an invalid action id"
            )
        first_id = self._action_id(first.request)
        second_id = self._action_id(second.request)
        if not observed or observed[0] != first_id:
            raise ProductAssertionFailure(
                f"{name} trajectory does not start at the selected root: "
                f"observed={observed!r}, expected_root={first_id!r}"
            )
        try:
            second_index = observed.index(second_id)
        except ValueError as error:
            raise ProductAssertionFailure(
                f"{name} trajectory does not contain the selected descendant: "
                f"observed={observed!r}, expected_descendant={second_id!r}"
            ) from error
        previous_body = self._request_body({"action_id": observed[0]})
        for index in range(1, second_index + 1):
            node = nodes[index]
            previous = nodes[index - 1]
            if node.get("trajectory_id") != trajectory_id:
                raise ProductAssertionFailure(
                    f"{name} trajectory id changes at {node.get('action_id')!r}"
                )
            if not isinstance(node.get("inference_version"), int):
                raise ProductAssertionFailure(
                    f"{name} trajectory node {node.get('action_id')!r} "
                    "has no inference version"
                )
            if node.get("parent_action_id") != previous.get("action_id"):
                raise ProductAssertionFailure(
                    f"{name} trajectory parent chain breaks at {node.get('action_id')!r}: "
                    f"parent={node.get('parent_action_id')!r}, "
                    f"expected={previous.get('action_id')!r}"
                )
            if node.get("trajectory_position") != index:
                raise ProductAssertionFailure(
                    f"{name} trajectory position at {node.get('action_id')!r} is "
                    f"{node.get('trajectory_position')!r}, expected {index}"
                )
            node_body = self._request_body({"action_id": node.get("action_id")})
            if not self._strict_prefix(
                self._history(previous_body), self._history(node_body)
            ):
                raise ProductAssertionFailure(
                    f"{name} trajectory history is not a strict prefix at "
                    f"{node.get('action_id')!r}"
                )
            previous_body = node_body

    def _require_descendant(
        self,
        name: str,
        ancestor: LlmCallPair,
        descendant: LlmCallPair,
        ancestor_lineage: dict[str, Any],
        descendant_lineage: dict[str, Any],
    ) -> None:
        if ancestor_lineage.get("trajectory_id") != descendant_lineage.get(
            "trajectory_id"
        ):
            raise ProductAssertionFailure(
                f"{name} descendant changed trajectory id"
            )
        if not self._strict_prefix(
            self._history(ancestor.request_body),
            self._history(descendant.request_body),
        ):
            raise ProductAssertionFailure(
                f"{name} ancestor does not have a strict history prefix"
            )

    @staticmethod
    def _history(body: dict[str, Any]) -> list[Any]:
        messages = body.get("messages")
        if not isinstance(messages, list):
            return []
        return [ClaudeTrajectoryAssertion._normalize_message(item) for item in messages]

    @staticmethod
    def _normalize_message(message: Any) -> Any:
        if not isinstance(message, dict):
            return message
        normalized = {
            key: value for key, value in message.items() if key != "cache_control"
        }
        content = message.get("content")
        if isinstance(content, str):
            normalized["content"] = [{"type": "text", "text": content}]
            return normalized
        if isinstance(content, dict):
            normalized["content"] = [
                ClaudeTrajectoryAssertion._normalize_content_item(content)
            ]
            return normalized
        if not isinstance(content, list):
            return normalized
        normalized["content"] = [
            ClaudeTrajectoryAssertion._normalize_content_item(item)
            for item in content
        ]
        return normalized

    @staticmethod
    def _normalize_content_item(item: Any) -> Any:
        if not isinstance(item, dict) or "cache_control" not in item:
            return item
        if item.get("type") not in {
            "text",
            "input_text",
            "output_text",
            "tool_result",
            "tool-result",
        }:
            return item
        return {key: value for key, value in item.items() if key != "cache_control"}
