from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from tests.v2.common.llm_trajectory.assertion import (
    ProductAssertionFailure,
    ScenarioPreconditionFailure,
    TrajectoryAssertionSupport,
)


class OfflineDelegationAssertion(TrajectoryAssertionSupport):
    def require(self, document: dict[str, Any]) -> int:
        for link in self._object_list(document, "links"):
            if link.get("role") == "agent.invocation.child_llm_request":
                raise ProductAssertionFailure("runtime persisted an offline-only child link")
        result = self._derive()
        if result.get("unavailable") != 0:
            raise ProductAssertionFailure(f"offline content reads failed: {result!r}")
        children: dict[str, set[str]] = {}
        edges_by_invocation: dict[str, dict[str, Any]] = {}
        for edge in result.get("edges", []):
            if edge.get("kind") != "delegation" or edge.get("confidence") != "derived":
                raise ProductAssertionFailure(f"unexpected offline edge: {edge!r}")
            if edge["invocation_id"] in edges_by_invocation:
                raise ProductAssertionFailure("duplicate offline invocation edge")
            edges_by_invocation[edge["invocation_id"]] = edge
            children.setdefault(edge["invocation_id"], set()).add(edge["target"])

        actions = self._object_list(document, "actions")
        by_id = {self._action_id(action): action for action in actions}
        parent_requests = {
            self._action_id(pair.response): {self._action_id(pair.request)}
            for pair in self._call_pairs(document, actions)
        }
        prompts: dict[tuple[str, int], str] = {}
        for action in actions:
            if action.get("kind") != "llm.response":
                continue
            raw = action.get("attributes", {}).get("llm.response.tool_calls_json")
            if not isinstance(raw, str):
                continue
            for ordinal, call in enumerate(json.loads(raw)):
                arguments = call.get("function", {}).get("arguments")
                if isinstance(arguments, str):
                    arguments = json.loads(arguments)
                prompt = arguments.get("prompt") if isinstance(arguments, dict) else None
                if isinstance(prompt, str):
                    prompts[(self._action_id(action), ordinal)] = prompt.strip()

        verified = 0
        for action in actions:
            if action.get("kind") != "agent.invocation":
                continue
            tool_id = action.get("attributes", {}).get("agent.invocation.tool_call_action_id")
            tool = by_id.get(tool_id, {}).get("attributes", {})
            ordinal = tool.get("llm.tool_call.ordinal")
            if ordinal is None:
                continue
            prompt = prompts.get((tool.get("llm.tool_call.response_action_id"), int(ordinal)), "")
            if len(prompt) <= 160:
                continue
            invocation_id = self._action_id(action)
            child_ids = children.get(invocation_id, set())
            if len(child_ids) != 1:
                raise ProductAssertionFailure(
                    f"long prompt invocation {invocation_id} ({len(prompt)} characters) "
                    "must have exactly one offline child LLM request link"
                )
            child_id = next(iter(child_ids))
            source = edges_by_invocation[invocation_id]["source"]
            if parent_requests.get(tool.get("llm.tool_call.response_action_id")) != {source}:
                raise ProductAssertionFailure("offline edge source is not the invocation's parent request")
            if by_id.get(child_id, {}).get("kind") != "llm.request":
                raise ProductAssertionFailure(f"offline child {child_id} is not a recorded request")
            lineage = self._environment.api.llm_request_lineage(self._trace_id, child_id).get("lineage", {})
            if (lineage.get("trajectory_position") != 0
                    or lineage.get("parent_action_id") is not None
                    or lineage.get("forked_from_action_id") is not None):
                raise ProductAssertionFailure(f"offline child {child_id} is not a trajectory root")
            if lineage.get("trajectory_id") == by_id.get(source, {}).get("attributes", {}).get("llm.request.trajectory_id"):
                raise ProductAssertionFailure("offline delegation stayed inside the parent trajectory")
            content = self._environment.api.llm_request_content(
                self._trace_id, child_id,
                max_bytes=self._environment.config.request_content_max_bytes,
            ).get("content", {})
            if content.get("truncated") is not False:
                raise ProductAssertionFailure(f"child {child_id} has incomplete request content")
            if not self._contains_prompt(json.loads(content["body_json"]), prompt):
                raise ProductAssertionFailure(f"child {child_id} does not contain the full prompt")
            verified += 1
        if verified < 2:
            raise ScenarioPreconditionFailure(
                f"expected two offline child prompts longer than 160 characters, found {verified}"
            )
        return verified

    def _derive(self) -> dict[str, Any]:
        config = self._environment.config
        prefix = "actrailweb listening on "
        log = (config.work_dir / "actrailweb.log").read_text(encoding="utf-8")
        urls = [line.removeprefix(prefix).split()[0] for line in log.splitlines() if line.startswith(prefix)]
        if len(urls) != 1:
            raise RuntimeError("expected one actual Web listening URL")
        result = self._environment.runtime.run(
            ["node", str(Path(__file__).with_name("offline.mjs")), str(config.repo),
             urls[0], str(self._trace_id), str(config.request_content_max_bytes),
             str(config.command_timeout_seconds)],
            timeout_seconds=config.command_timeout_seconds,
            echo=False,
        )
        if result.returncode != 0:
            raise ProductAssertionFailure(f"offline Web correlation failed: {result.stderr[-2000:]}")
        return json.loads(result.stdout)

    @classmethod
    def _contains_prompt(cls, value: Any, prompt: str) -> bool:
        if isinstance(value, str):
            return value.strip() == prompt
        if isinstance(value, dict):
            return any(cls._contains_prompt(item, prompt) for item in value.values())
        if isinstance(value, list):
            return any(cls._contains_prompt(item, prompt) for item in value)
        return False
