from __future__ import annotations

from dataclasses import dataclass
from math import isclose
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


@dataclass(frozen=True)
class TrajectoryGraphEvidence:
    node_count: int
    edge_count: int
    trajectory_count: int
    append_count: int
    fork_count: int
    duplicate_count: int


@dataclass(frozen=True)
class RealAnalysisEvidence:
    independent_context_count: int
    continuous_context_count: int


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

    def require_graph(
        self,
        evidence: ProjectSubagentEvidence,
    ) -> TrajectoryGraphEvidence:
        document = self._environment.api.llm_trajectory_graph(self._trace_id)
        if document.get("trace_id") != self._trace_id:
            raise ProductAssertionFailure(
                "trajectory graph returned the wrong trace id: "
                f"{document.get('trace_id')!r}"
            )
        if document.get("partial") is not False:
            raise ProductAssertionFailure("trajectory graph is unexpectedly partial")
        expected_capabilities = {
            "strict_prefix_edges": True,
            "related_edges": False,
            "compaction_detection": False,
        }
        if document.get("capabilities") != expected_capabilities:
            raise ProductAssertionFailure(
                "trajectory graph capabilities changed: "
                f"{document.get('capabilities')!r}"
            )

        requests = {
            self._action_id(pair.request): pair.request for pair in evidence.pairs
        }
        lineages = {
            action_id: self._require_lineage(action_id)
            for action_id in requests
        }
        nodes = self._object_list(document, "nodes")
        nodes_by_id = {
            node_id: node
            for node in nodes
            if isinstance((node_id := node.get("id")), str) and node_id
        }
        if len(nodes_by_id) != len(nodes):
            raise ProductAssertionFailure(
                "trajectory graph contains a node with a missing or duplicate id"
            )
        if set(nodes_by_id) != set(requests):
            raise ProductAssertionFailure(
                "trajectory graph node set differs from persisted requests: "
                f"observed={sorted(nodes_by_id)}, expected={sorted(requests)}"
            )
        for action_id, request in requests.items():
            self._require_graph_node(
                action_id,
                nodes_by_id[action_id],
                request,
                lineages[action_id],
            )
        observed_node_order = [str(node["id"]) for node in nodes]
        expected_node_order = [
            str(node["id"])
            for node in sorted(
                nodes,
                key=lambda node: (
                    len(str(node["start_time_unix_nanos"])),
                    str(node["start_time_unix_nanos"]),
                    str(node["id"]),
                ),
            )
        ]
        if observed_node_order != expected_node_order:
            raise ProductAssertionFailure("trajectory graph nodes are not stably ordered")

        expected_edges: list[dict[str, str]] = []
        for action_id, lineage in lineages.items():
            parent = lineage.get("parent_action_id")
            forked_from = lineage.get("forked_from_action_id")
            if isinstance(parent, str) and parent:
                expected_edges.append(
                    {
                        "source": parent,
                        "target": action_id,
                        "kind": "append",
                        "confidence": "derived",
                    }
                )
            elif isinstance(forked_from, str) and forked_from:
                expected_edges.append(
                    {
                        "source": forked_from,
                        "target": action_id,
                        "kind": "fork",
                        "confidence": "derived",
                    }
                )
        expected_edges.sort(key=lambda edge: (edge["target"], edge["source"]))
        edges = self._object_list(document, "edges")
        if edges != expected_edges:
            raise ProductAssertionFailure(
                "trajectory graph edges differ from persisted lineage: "
                f"observed={edges!r}, expected={expected_edges!r}"
            )

        transitions = [lineage.get("transition") for lineage in lineages.values()]
        trajectory_count = len(
            {str(lineage["trajectory_id"]) for lineage in lineages.values()}
        )
        append_count = transitions.count("append")
        fork_count = transitions.count("fork_root")
        duplicate_count = transitions.count("duplicate_root")
        stats = document.get("stats")
        if not isinstance(stats, dict):
            raise ProductAssertionFailure("trajectory graph has no stats object")
        expected_counts = {
            "node_count": len(nodes),
            "trajectory_count": trajectory_count,
            "append_count": append_count,
            "fork_count": fork_count,
            "duplicate_count": duplicate_count,
        }
        for key, expected in expected_counts.items():
            if stats.get(key) != expected:
                raise ProductAssertionFailure(
                    f"trajectory graph stat {key}={stats.get(key)!r}, "
                    f"expected {expected}"
                )
        self._require_ratio(
            stats,
            "strongly_linked_node_ratio",
            (append_count + fork_count) / len(nodes),
        )
        self._require_ratio(
            stats,
            "duplicate_node_ratio",
            duplicate_count / len(nodes),
        )
        return TrajectoryGraphEvidence(
            node_count=len(nodes),
            edge_count=len(edges),
            trajectory_count=trajectory_count,
            append_count=append_count,
            fork_count=fork_count,
            duplicate_count=duplicate_count,
        )

    def require_real_analysis_scenario(self) -> RealAnalysisEvidence:
        document = self._environment.api.llm_trajectory_graph(self._trace_id)
        nodes = self._object_list(document, "nodes")
        trajectories: dict[str, list[dict[str, Any]]] = {}
        for node in nodes:
            trajectory_id = node.get("trajectory_id")
            if not isinstance(trajectory_id, str) or not trajectory_id:
                raise ProductAssertionFailure("graph node has no trajectory id")
            trajectories.setdefault(trajectory_id, []).append(node)
        independent_context_count = sum(
            any(node.get("trajectory_position") == 0 for node in members)
            for members in trajectories.values()
        )
        if independent_context_count < 3:
            raise ScenarioPreconditionFailure(
                "sorting-subagent scenario produced fewer than three "
                "independent contexts"
            )
        continuous_context_count = sum(
            len(members) >= 2 for members in trajectories.values()
        )
        if continuous_context_count < 1:
            raise ScenarioPreconditionFailure(
                "sorting-subagent scenario produced no continuous context"
            )
        increased = False
        for members in trajectories.values():
            ordered = sorted(members, key=lambda node: node["trajectory_position"])
            counts = [node.get("tool_result_count") for node in ordered]
            observed = [count for count in counts if isinstance(count, int)]
            if observed != sorted(observed):
                raise ProductAssertionFailure(
                    "tool result count decreased across a strict append edge"
                )
            increased |= any(
                right > left for left, right in zip(observed, observed[1:])
            )
        if not increased:
            raise ScenarioPreconditionFailure(
                "sorting-subagent scenario did not accumulate tool results"
            )
        order = [str(node["trajectory_id"]) for node in nodes]
        interleaved = False
        for trajectory_id in trajectories:
            positions = [
                index
                for index, observed in enumerate(order)
                if observed == trajectory_id
            ]
            if any(
                other != trajectory_id
                for other in order[positions[0] + 1 : positions[-1]]
            ):
                interleaved = True
                break
        if not interleaved:
            raise ScenarioPreconditionFailure(
                "sorting-subagent scenario did not interleave trajectories in time"
            )
        incomplete = [
            str(node.get("id"))
            for node in nodes
            if node.get("status") != "success"
            or node.get("completeness") != "complete"
        ]
        if incomplete:
            raise ProductAssertionFailure(
                "successful sorting scenario contains incomplete/error nodes: "
                + ", ".join(incomplete)
            )
        return RealAnalysisEvidence(
            independent_context_count=independent_context_count,
            continuous_context_count=continuous_context_count,
        )

    def _require_lineage(self, action_id: str) -> dict[str, Any]:
        document = self._environment.api.llm_request_lineage(
            self._trace_id,
            action_id,
        )
        lineage = document.get("lineage")
        if not isinstance(lineage, dict):
            raise ProductAssertionFailure(
                f"request {action_id} has no persisted lineage"
            )
        return lineage

    def _require_graph_node(
        self,
        action_id: str,
        node: dict[str, Any],
        request: dict[str, Any],
        lineage: dict[str, Any],
    ) -> None:
        expected = {
            "trajectory_id": lineage.get("trajectory_id"),
            "trajectory_position": lineage.get("trajectory_position"),
            "transition": lineage.get("transition"),
            "start_reason": lineage.get("start_reason"),
            "inference_version": lineage.get("inference_version"),
            "status": request.get("status"),
            "completeness": request.get("completeness"),
            "compaction_boundary": lineage.get("start_reason")
            == "context_rewrite_or_compression",
        }
        attributes = request.get("attributes")
        if not isinstance(attributes, dict):
            raise ScenarioPreconditionFailure(
                f"request {action_id} has no attributes"
            )
        expected.update(
            {
                "model": attributes.get("llm.request.model"),
                "classifier_id": attributes.get("llm.request.classifier_id"),
                "block_count": self._optional_count(attributes, "block_count"),
                "user_message_count": self._optional_count(
                    attributes, "user_message_count"
                ),
                "tool_result_count": self._optional_count(
                    attributes, "tool_result_count"
                ),
            }
        )
        process = request.get("process")
        if isinstance(process, dict) and "process_id" in process:
            expected["process"] = {"process_id": process["process_id"]}
        start_nanos = request.get("start_time_unix_nanos")
        if start_nanos is not None:
            expected["start_time_unix_nanos"] = str(start_nanos)
        for key, value in expected.items():
            if node.get(key) != value:
                raise ProductAssertionFailure(
                    f"trajectory graph node {action_id} field {key}="
                    f"{node.get(key)!r}, expected {value!r}"
                )
        nanos = node.get("start_time_unix_nanos")
        if not isinstance(nanos, str) or not nanos.isdigit():
            raise ProductAssertionFailure(
                f"trajectory graph node {action_id} has invalid nanosecond time"
            )

    @staticmethod
    def _optional_count(attributes: dict[str, Any], suffix: str) -> int | None:
        value = attributes.get(f"llm.request.{suffix}")
        if value is None:
            return None
        try:
            return int(value)
        except (TypeError, ValueError) as error:
            raise ScenarioPreconditionFailure(
                f"llm.request.{suffix} is not an integer: {value!r}"
            ) from error

    @staticmethod
    def _require_ratio(stats: dict[str, Any], key: str, expected: float) -> None:
        value = stats.get(key)
        if not isinstance(value, (int, float)) or not isclose(
            float(value), expected, rel_tol=1e-12, abs_tol=1e-12
        ):
            raise ProductAssertionFailure(
                f"trajectory graph stat {key}={value!r}, expected {expected}"
            )

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
