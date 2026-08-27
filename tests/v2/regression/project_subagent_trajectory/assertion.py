from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from tests.v2.common.llm_trajectory.assertion import (
    LlmCallPair,
    ProductAssertionFailure,
    ScenarioPreconditionFailure,
    TrajectoryAssertionSupport,
)
from tests.v2.common.llm_trajectory.environment import (
    TrajectoryTestEnvironment,
)


@dataclass(frozen=True)
class ProjectSubagentEvidence:
    pairs: tuple[LlmCallPair, ...]

    @property
    def request_pairs(self) -> tuple[tuple[str, LlmCallPair], ...]:
        return tuple(
            (f"exchange_{index}", pair) for index, pair in enumerate(self.pairs)
        )


class ProjectSubagentTrajectoryAssertion(TrajectoryAssertionSupport):
    def __init__(
        self,
        environment: TrajectoryTestEnvironment,
        trace_id: int,
    ):
        super().__init__(environment, trace_id)

    def require_scenario(
        self,
        document: dict[str, Any],
    ) -> ProjectSubagentEvidence:
        actions = self._object_list(document, "actions")
        self._require_terminal_llm_actions(actions)
        return ProjectSubagentEvidence(pairs=tuple(self._call_pairs(document, actions)))

    def require_trajectories(
        self,
        evidence: ProjectSubagentEvidence,
    ) -> dict[str, str]:
        groups: dict[str, list[tuple[LlmCallPair, dict[str, Any]]]] = {}
        action_trajectory_ids: dict[str, str] = {}
        for pair in evidence.pairs:
            action_id = self._action_id(pair.request)
            lineage_document = self._environment.api.llm_request_lineage(
                self._trace_id,
                action_id,
            )
            lineage = lineage_document.get("lineage")
            if not isinstance(lineage, dict):
                raise ProductAssertionFailure(
                    f"request {action_id} has no persisted lineage"
                )
            trajectory_id = lineage.get("trajectory_id")
            attributes = pair.request.get("attributes")
            if not isinstance(trajectory_id, str) or not trajectory_id:
                raise ProductAssertionFailure(
                    f"request {action_id} has no trajectory id"
                )
            if not isinstance(attributes, dict):
                raise ProductAssertionFailure(f"request {action_id} has no attributes")
            if attributes.get(self._TRAJECTORY_ID) != trajectory_id:
                raise ProductAssertionFailure(
                    f"request {action_id} action/lineage trajectory mismatch"
                )
            action_trajectory_ids[action_id] = trajectory_id
            groups.setdefault(trajectory_id, []).append((pair, lineage))

        if len(groups) < 2:
            raise ScenarioPreconditionFailure(
                "multi-agent scenario produced fewer than two trajectories"
            )
        for trajectory_id, members in groups.items():
            self._require_linear_trajectory(trajectory_id, members)
        return action_trajectory_ids

    def _require_linear_trajectory(
        self,
        trajectory_id: str,
        members: list[tuple[LlmCallPair, dict[str, Any]]],
    ) -> tuple[tuple[LlmCallPair, dict[str, Any]], ...]:
        ordered = tuple(
            sorted(
                members,
                key=lambda member: member[1].get("trajectory_position", -1),
            )
        )
        for position, (pair, lineage) in enumerate(ordered):
            action_id = self._action_id(pair.request)
            if lineage.get("trajectory_position") != position:
                raise ProductAssertionFailure(
                    f"trajectory {trajectory_id} position is not continuous at {action_id}"
                )
            expected_parent = (
                None
                if position == 0
                else self._action_id(ordered[position - 1][0].request)
            )
            if lineage.get("parent_action_id") != expected_parent:
                raise ProductAssertionFailure(
                    f"trajectory {trajectory_id} parent chain breaks at {action_id}"
                )
        endpoint = self._environment.api.llm_request_trajectory(
            self._trace_id,
            trajectory_id,
        )
        nodes = endpoint.get("nodes")
        expected_ids = [self._action_id(pair.request) for pair, _ in ordered]
        observed_ids = (
            [node.get("action_id") for node in nodes]
            if isinstance(nodes, list) and all(isinstance(node, dict) for node in nodes)
            else None
        )
        if observed_ids != expected_ids:
            raise ProductAssertionFailure(
                f"trajectory endpoint differs from actions for {trajectory_id}: "
                f"observed={observed_ids!r}, expected={expected_ids!r}"
            )
        return ordered

    @staticmethod
    def _require_terminal_llm_actions(actions: list[dict[str, Any]]) -> None:
        for action in actions:
            if action.get("kind") not in {"llm.call", "llm.request", "llm.response"}:
                continue
            if action.get("status") == "in_progress":
                raise ScenarioPreconditionFailure(
                    f"LLM action {action.get('action_id')} is still in_progress"
                )
