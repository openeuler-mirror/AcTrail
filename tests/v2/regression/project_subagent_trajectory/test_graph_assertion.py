from __future__ import annotations

import copy
import unittest
from types import SimpleNamespace
from unittest.mock import patch
from typing import Any

from tests.v2.common.llm_trajectory.assertion import (
    LlmCallPair,
    ProductAssertionFailure,
)
from tests.v2.regression.project_subagent_trajectory.agent import (
    ProjectSubagentAgent,
)
from tests.v2.regression.project_subagent_trajectory.case import (
    ProjectSubagentTrajectoryCase,
)
from tests.v2.regression.project_subagent_trajectory.assertion import (
    ProjectSubagentEvidence,
    ProjectSubagentTrajectoryAssertion,
)


class _FakeApi:
    def __init__(
        self,
        graph: dict[str, Any],
        lineages: dict[str, dict[str, Any]],
    ):
        self.graph = graph
        self.lineages = lineages

    def llm_trajectory_graph(self, trace_id: int) -> dict[str, Any]:
        return copy.deepcopy(self.graph)

    def llm_request_lineage(
        self,
        trace_id: int,
        action_id: str,
    ) -> dict[str, Any]:
        return {"lineage": copy.deepcopy(self.lineages[action_id])}


def _request(
    action_id: str,
    nanos: str,
    process_id: int,
    *,
    tool_result_count: str | None,
) -> dict[str, Any]:
    attributes = {
        "llm.request.model": "fixture-model",
        "llm.request.classifier_id": "fixture-classifier",
        "llm.request.block_count": "2",
        "llm.request.user_message_count": "1",
    }
    if tool_result_count is not None:
        attributes["llm.request.tool_result_count"] = tool_result_count
    return {
        "action_id": action_id,
        "status": "success",
        "completeness": "complete",
        "start_time_unix_nanos": nanos,
        "process": {"process_id": process_id},
        "attributes": attributes,
    }


def _lineage(
    trajectory_id: str,
    position: int,
    transition: str,
    *,
    parent: str | None = None,
    forked_from: str | None = None,
) -> dict[str, Any]:
    return {
        "trajectory_id": trajectory_id,
        "trajectory_position": position,
        "transition": transition,
        "start_reason": "first_observed_request"
        if position == 0
        else "strict_prefix_append",
        "inference_version": 1,
        "parent_action_id": parent,
        "forked_from_action_id": forked_from,
    }


class TrajectoryGraphAssertionTest(unittest.TestCase):
    def setUp(self) -> None:
        self.requests = {
            "request-a": _request("request-a", "100", 11, tool_result_count=None),
            "request-b": _request("request-b", "200", 11, tool_result_count="0"),
            "request-c": _request("request-c", "300", 12, tool_result_count="1"),
        }
        self.lineages = {
            "request-a": _lineage("trajectory-main", 0, "new_root"),
            "request-b": _lineage(
                "trajectory-main",
                1,
                "append",
                parent="request-a",
            ),
            "request-c": _lineage(
                "trajectory-fork",
                0,
                "fork_root",
                forked_from="request-a",
            ),
        }
        nodes = [
            self._node(action_id)
            for action_id in ("request-a", "request-b", "request-c")
        ]
        self.graph = {
            "trace_id": 42,
            "partial": False,
            "nodes": nodes,
            "edges": [
                {
                    "source": "request-a",
                    "target": "request-b",
                    "kind": "append",
                    "confidence": "derived",
                },
                {
                    "source": "request-a",
                    "target": "request-c",
                    "kind": "fork",
                    "confidence": "derived",
                },
            ],
            "stats": {
                "node_count": 3,
                "trajectory_count": 2,
                "append_count": 1,
                "fork_count": 1,
                "duplicate_count": 0,
                "strongly_linked_node_ratio": 2 / 3,
                "duplicate_node_ratio": 0.0,
            },
            "capabilities": {
                "strict_prefix_edges": True,
                "related_edges": False,
                "compaction_detection": False,
            },
        }

    def test_accepts_graph_derived_from_requests_and_lineages(self) -> None:
        result = self._assertion().require_graph(self._evidence())

        self.assertEqual(result.node_count, 3)
        self.assertEqual(result.edge_count, 2)
        self.assertEqual(result.trajectory_count, 2)
        self.assertEqual(result.append_count, 1)
        self.assertEqual(result.fork_count, 1)

    def test_rejects_missing_lineage_edge(self) -> None:
        self.graph["edges"].pop()

        with self.assertRaisesRegex(
            ProductAssertionFailure,
            "edges differ from persisted lineage",
        ):
            self._assertion().require_graph(self._evidence())

    def test_rejects_null_zero_count_regression(self) -> None:
        self.graph["nodes"][1]["tool_result_count"] = None

        with self.assertRaisesRegex(
            ProductAssertionFailure,
            "tool_result_count",
        ):
            self._assertion().require_graph(self._evidence())

    def test_accepts_real_sorting_analysis_shape(self) -> None:
        graph = copy.deepcopy(self.graph)
        graph["nodes"][0]["tool_result_count"] = 0
        graph["nodes"][1]["tool_result_count"] = 2
        third_root = copy.deepcopy(graph["nodes"][2])
        third_root.update(
            {
                "id": "request-d",
                "trajectory_id": "trajectory-other",
                "trajectory_position": 0,
                "transition": "new_root",
                "tool_result_count": 0,
            }
        )
        graph["nodes"] = [
            graph["nodes"][0],
            third_root,
            graph["nodes"][2],
            graph["nodes"][1],
        ]
        self.graph = graph

        result = self._assertion().require_real_analysis_scenario()

        self.assertEqual(result.independent_context_count, 3)
        self.assertEqual(result.continuous_context_count, 1)

    def test_rejects_incomplete_real_sorting_node(self) -> None:
        graph = copy.deepcopy(self.graph)
        graph["nodes"][0]["tool_result_count"] = 0
        graph["nodes"][1]["tool_result_count"] = 2
        third_root = copy.deepcopy(graph["nodes"][2])
        third_root.update(
            {
                "id": "request-d",
                "trajectory_id": "trajectory-other",
                "trajectory_position": 0,
                "transition": "new_root",
            }
        )
        graph["nodes"] = [
            graph["nodes"][0],
            third_root,
            graph["nodes"][2],
            graph["nodes"][1],
        ]
        graph["nodes"][1]["completeness"] = "incomplete"
        self.graph = graph

        with self.assertRaisesRegex(
            ProductAssertionFailure,
            "incomplete/error nodes",
        ):
            self._assertion().require_real_analysis_scenario()

    def _assertion(self) -> ProjectSubagentTrajectoryAssertion:
        environment = SimpleNamespace(api=_FakeApi(self.graph, self.lineages))
        return ProjectSubagentTrajectoryAssertion(environment, 42)

    def _evidence(self) -> ProjectSubagentEvidence:
        return ProjectSubagentEvidence(
            pairs=tuple(
                LlmCallPair(request=request, response={})
                for request in self.requests.values()
            )
        )

    def _node(self, action_id: str) -> dict[str, Any]:
        request = self.requests[action_id]
        lineage = self.lineages[action_id]
        attributes = request["attributes"]
        return {
            "id": action_id,
            "trajectory_id": lineage["trajectory_id"],
            "trajectory_position": lineage["trajectory_position"],
            "transition": lineage["transition"],
            "start_reason": lineage["start_reason"],
            "inference_version": lineage["inference_version"],
            "start_time": int(request["start_time_unix_nanos"]) // 1_000_000,
            "start_time_unix_nanos": request["start_time_unix_nanos"],
            "model": attributes["llm.request.model"],
            "classifier_id": attributes["llm.request.classifier_id"],
            "block_count": int(attributes["llm.request.block_count"]),
            "user_message_count": int(
                attributes["llm.request.user_message_count"]
            ),
            "tool_result_count": (
                int(attributes["llm.request.tool_result_count"])
                if "llm.request.tool_result_count" in attributes
                else None
            ),
            "process": request["process"],
            "status": request["status"],
            "completeness": request["completeness"],
            "compaction_boundary": False,
        }


class _SelectionContext:
    def __init__(self, available: set[str]):
        self.available = available
        self.checked: list[str] = []

    def report_progress(self, stage: str, message: str) -> None:
        pass

    def check_agent_availability(
        self,
        name: str,
        binary: object,
        environment: object,
    ) -> bool:
        self.checked.append(name)
        return name in self.available


class AgentCandidateTest(unittest.TestCase):
    def test_auto_mode_uses_fallback_order(self) -> None:
        self.assertEqual(
            ProjectSubagentAgent.candidates(None),
            ("opencode", "claude", "xiaoo"),
        )

    def test_explicit_mode_only_uses_requested_agent(self) -> None:
        self.assertEqual(ProjectSubagentAgent.candidates("claude"), ("claude",))

    def test_selection_skips_missing_and_unavailable_candidates(self) -> None:
        case = object.__new__(ProjectSubagentTrajectoryCase)
        case._config = SimpleNamespace(agent_binary=None)
        context = _SelectionContext({"xiaoo"})

        def resolve(name: str, discovery: object) -> object | None:
            if name == "opencode":
                return None
            return SimpleNamespace(name=name, binary=name, environment={})

        with patch.object(ProjectSubagentAgent, "resolve", side_effect=resolve):
            selected = case._select_agent(context, SimpleNamespace())

        self.assertEqual(selected.name, "xiaoo")
        self.assertEqual(context.checked, ["claude", "xiaoo"])

    def test_selection_returns_none_when_every_agent_is_missing(self) -> None:
        case = object.__new__(ProjectSubagentTrajectoryCase)
        case._config = SimpleNamespace(agent_binary=None)
        context = _SelectionContext(set())

        with patch.object(ProjectSubagentAgent, "resolve", return_value=None):
            selected = case._select_agent(context, SimpleNamespace())

        self.assertIsNone(selected)


if __name__ == "__main__":
    unittest.main()
