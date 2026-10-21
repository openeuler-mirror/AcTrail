from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from tests.v2.common.llm_trajectory.assertion import (
    ProductAssertionFailure,
    ScenarioPreconditionFailure,
    TrajectoryAssertionSupport,
)

from tests.v2.regression.project_subagent_trajectory.constants import CASE_CONSTANTS


class PersistedTrajectoryWebAssertion(TrajectoryAssertionSupport):
    def require(self) -> int:
        result = self._layout()
        graph = result["graph"]
        layout = result["layout"]
        nodes = self._object_list(graph, "nodes")
        edges = self._object_list(graph, "edges")
        if not nodes:
            raise ScenarioPreconditionFailure("real agent produced no trajectory nodes")
        if {node["id"] for node in layout["nodes"]} != {node["id"] for node in nodes}:
            raise ProductAssertionFailure("Web layout lost persisted request nodes")
        expected = {(edge["source"], edge["target"], edge["kind"]) for edge in edges}
        observed = {(edge["source"], edge["target"], edge["kind"]) for edge in layout["edges"]}
        if observed != expected:
            raise ProductAssertionFailure("Web layout changed persisted lineage edges")
        for edge in edges:
            lineage = self._environment.api.llm_request_lineage(
                self._trace_id, edge["target"]
            ).get("lineage", {})
            field = {"append": "parent_action_id", "fork": "forked_from_action_id"}.get(edge["kind"])
            if field is None or lineage.get(field) != edge["source"]:
                raise ProductAssertionFailure(f"Web edge lacks persisted lineage: {edge!r}")
        return len(nodes)

    def _layout(self) -> dict[str, Any]:
        config = self._environment.config
        prefix = "actrailweb listening on "
        log = (config.work_dir / "actrailweb.log").read_text(encoding="utf-8")
        urls = [line.removeprefix(prefix).split()[0] for line in log.splitlines() if line.startswith(prefix)]
        if len(urls) != 1:
            raise RuntimeError("expected one actual Web listening URL")
        result = self._environment.runtime.run(
            ["node", str(Path(__file__).with_name("trajectory.mjs")), str(config.repo),
             urls[0], str(self._trace_id),
             str(config.command_timeout_seconds)],
            timeout_seconds=config.command_timeout_seconds,
            echo=False,
        )
        if result.returncode != 0:
            raise ProductAssertionFailure(f"persisted Web trajectory layout failed: {result.stderr[-CASE_CONSTANTS.command_diagnostic_tail_characters:]}")
        return json.loads(result.stdout)

