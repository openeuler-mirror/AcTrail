from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from tests.v2.common.actrail_runtime import ActrailRuntime, CommandResult

from .action_contract import McpActionContract
from .probe import McpProbeSpec


@dataclass(frozen=True)
class McpSemanticSummary:
    tool_calls: int


class McpTraceAssertion:
    _TRACE_PATTERN = re.compile(r"trace trace-(\d+) entered Active")
    _MCP_KINDS = frozenset(
        {
            "mcp.tool_call",
            "mcp.request",
            "mcp.response",
            "mcp.stdin",
            "mcp.stdout",
        }
    )

    def __init__(self, runtime: ActrailRuntime) -> None:
        self._runtime = runtime
        self._contract = McpActionContract()

    def require_trace_id(self, launch: CommandResult) -> int:
        trace_ids = [
            int(value) for value in self._TRACE_PATTERN.findall(launch.output)
        ]
        if len(trace_ids) != 1:
            raise AssertionError(
                f"MCP launch must report exactly one trace id; found {trace_ids}"
            )
        return trace_ids[0]

    def require_finalized_semantics(
        self,
        trace_id: int,
        expected_calls: tuple[McpProbeSpec, ...],
    ) -> McpSemanticSummary:
        if not expected_calls:
            raise ValueError("at least one expected MCP call is required")
        self._require_clean_trace(trace_id)
        document = self._read_json(
            self._runtime.viewer_command(
                "--output-format",
                "json",
                "actions",
                "--trace-id",
                str(trace_id),
            )
        )
        actions = self._object_list(document, "actions")
        links = self._object_list(document, "links")
        actions_by_id = self._index_actions(actions)
        mcp_actions = [
            action for action in actions if action.get("kind") in self._MCP_KINDS
        ]
        expected_action_count = len(expected_calls) * 5
        if len(mcp_actions) != expected_action_count:
            raise AssertionError(
                f"trace-{trace_id} must contain exactly {expected_action_count} "
                f"MCP actions; found {len(mcp_actions)}: "
                f"{self._contract.report(mcp_actions)}"
            )
        for action in mcp_actions:
            self._contract.require_terminal(action)
        roots = [
            action for action in mcp_actions if action.get("kind") == "mcp.tool_call"
        ]
        if len(roots) != len(expected_calls):
            raise AssertionError(
                f"trace-{trace_id} must contain exactly {len(expected_calls)} "
                f"mcp.tool_call action(s); found {self._contract.report(roots)}"
            )
        remaining_roots = {action["action_id"]: action for action in roots}
        for expected in expected_calls:
            matching = [
                action
                for action in remaining_roots.values()
                if self._contract.root_identity(action)
                == (
                    expected.server_name,
                    expected.tool_name,
                    "stdio",
                )
            ]
            if len(matching) != 1:
                raise AssertionError(
                    f"expected exactly one semantic root for {expected.tool_id} "
                    f"over stdio; found {self._contract.report(matching)} among "
                    f"{self._contract.report(list(remaining_roots.values()))}"
                )
            root = matching[0]
            self._require_tool_call_graph(
                root,
                expected,
                actions_by_id,
                links,
            )
            del remaining_roots[root["action_id"]]
        if remaining_roots:
            raise AssertionError(
                "unexpected MCP tool-call roots: "
                f"{self._contract.report(list(remaining_roots.values()))}"
            )
        self._require_payload_integrity(
            trace_id,
            actions,
            links,
        )
        return McpSemanticSummary(len(expected_calls))

    def require_diagnostic(
        self,
        trace_id: int,
        code: str,
        reason: str,
    ) -> None:
        output = self._runtime.run_checked(
            self._runtime.viewer_command(
                "diagnostics",
                "--trace-id",
                str(trace_id),
            ),
            echo=False,
        ).stdout
        expected = f"MCP stdio observation {code}: reason={reason}"
        if expected not in output:
            raise AssertionError(
                f"trace-{trace_id} has no expected MCP diagnostic "
                f"{expected!r}: {output[-4000:]}"
            )

    def _require_clean_trace(self, trace_id: int) -> None:
        document = self._read_json(
            self._runtime.viewer_command(
                "--output-format",
                "json",
                "traces",
            )
        )
        traces = [
            trace
            for trace in self._object_list(document, "traces")
            if trace.get("trace_id_raw") == trace_id
        ]
        if len(traces) != 1:
            raise AssertionError(
                f"trace-{trace_id} must appear exactly once; found {traces}"
            )
        trace = traces[0]
        if trace.get("state") != "Exited" or trace.get("health") != "Clean":
            raise AssertionError(
                f"trace-{trace_id} must be Exited/Clean after daemon shutdown; "
                f"state={trace.get('state')} health={trace.get('health')}"
            )

    def _require_tool_call_graph(
        self,
        root: dict[str, Any],
        expected: McpProbeSpec,
        actions_by_id: dict[str, dict[str, Any]],
        links: list[dict[str, Any]],
    ) -> None:
        root_id = root["action_id"]
        attributes = self._contract.attributes(root)
        self._contract.require_identity(root, expected)
        self._contract.require_attributes(
            root,
            {
                "mcp.execution.status": "success",
                "mcp.tool.id": expected.tool_name,
                "llm.tool_call.name": expected.tool_id,
            },
        )
        for key in (
            "mcp.request.id",
            "llm.response.action_id",
            "llm.tool_call.id",
        ):
            if not attributes.get(key):
                raise AssertionError(f"{root_id} has no nonempty {key}")
        llm_response_id = attributes["llm.response.action_id"]
        llm_response = actions_by_id.get(llm_response_id)
        if llm_response is None or llm_response.get("kind") != "llm.response":
            raise AssertionError(
                f"{root_id} references invalid llm.response action "
                f"{llm_response_id!r}"
            )
        command = self._linked_parent(
            root_id,
            "command.contains_mcp_tool_call",
            actions_by_id,
            links,
        )
        if command.get("kind") != "command.invocation":
            raise AssertionError(
                f"{root_id} MCP parent must be command.invocation; "
                f"found {command.get('kind')}"
            )
        request = self._linked_child(
            root_id,
            "mcp.tool_call.request",
            "mcp.request",
            actions_by_id,
            links,
        )
        response = self._linked_child(
            root_id,
            "mcp.tool_call.response",
            "mcp.response",
            actions_by_id,
            links,
        )
        outbound = self._linked_child(
            request["action_id"],
            "mcp.request.stdout",
            "mcp.stdout",
            actions_by_id,
            links,
        )
        inbound = self._linked_child(
            response["action_id"],
            "mcp.response.stdin",
            "mcp.stdin",
            actions_by_id,
            links,
        )
        related = [
            action
            for action in actions_by_id.values()
            if self._contract.attributes(action).get("mcp.tool_call.action_id")
            == root_id
        ]
        expected_kinds = {
            "mcp.request",
            "mcp.response",
            "mcp.stdout",
            "mcp.stdin",
        }
        if {action.get("kind") for action in related} != expected_kinds or len(
            related
        ) != 4:
            raise AssertionError(
                f"{root_id} must have exactly the four expected MCP children; "
                f"found {self._contract.report(related)}"
            )
        for action in (request, response, outbound, inbound):
            self._contract.require_identity(action, expected)
            self._contract.require_attributes(
                action,
                {"mcp.tool_call.action_id": root_id},
            )
        self._contract.require_reference(
            attributes,
            "mcp.request.action_id",
            request,
        )
        self._contract.require_reference(
            attributes,
            "mcp.response.action_id",
            response,
        )
        request_attributes = self._contract.attributes(request)
        response_attributes = self._contract.attributes(response)
        self._contract.require_reference(
            attributes,
            "mcp.stdout.action_id",
            outbound,
        )
        self._contract.require_reference(
            attributes,
            "mcp.stdin.action_id",
            inbound,
        )
        self._contract.require_reference(
            request_attributes,
            "mcp.stdout.action_id",
            outbound,
        )
        self._contract.require_reference(
            response_attributes,
            "mcp.stdin.action_id",
            inbound,
        )
        self._contract.require_reference(
            self._contract.attributes(outbound),
            "mcp.request.action_id",
            request,
        )
        self._contract.require_reference(
            self._contract.attributes(inbound),
            "mcp.response.action_id",
            response,
        )
        request_id = attributes["mcp.request.id"]
        self._contract.require_attributes(
            outbound,
            {
                "mcp.message.method": "tools/call",
                "mcp.message.direction": "outbound",
                "mcp.message.id": request_id,
                "mcp.tool_call.request_id": request_id,
            },
        )
        self._contract.require_attributes(
            inbound,
            {
                "mcp.message.direction": "inbound",
                "mcp.message.id": request_id,
                "mcp.tool_call.request_id": request_id,
            },
        )

    def _require_payload_integrity(
        self,
        trace_id: int,
        actions: list[dict[str, Any]],
        links: list[dict[str, Any]],
    ) -> None:
        document = self._read_json(
            self._runtime.viewer_command(
                "--output-format",
                "json",
                "payloads",
                "--trace-id",
                str(trace_id),
            )
        )
        payloads = self._object_list(document, "payloads")
        payload_ids: set[int] = set()
        for payload in payloads:
            segment_id = payload.get("segment_id_raw")
            if type(segment_id) is not int or segment_id <= 0:
                raise AssertionError(
                    f"trace-{trace_id} has invalid payload segment id: {payload}"
                )
            if segment_id in payload_ids:
                raise AssertionError(
                    f"trace-{trace_id} has duplicate payload segment id "
                    f"{segment_id}"
                )
            payload_ids.add(segment_id)

        stdout_payloads = [
            payload
            for payload in payloads
            if payload.get("protocol_hint") == "stdout"
        ]
        if stdout_payloads:
            raise AssertionError(
                "default stdout_storage_mode=drop must not persist stdout "
                f"payloads: {stdout_payloads}"
            )
        if not any(
            payload.get("protocol_hint") == "stdin"
            for payload in payloads
        ):
            raise AssertionError(
                f"trace-{trace_id} has no persisted MCP stdin payload"
            )

        for action in actions:
            self._require_persisted_payload_evidence(
                f"action {action.get('action_id')}",
                action.get("evidence"),
                payload_ids,
            )
        for link in links:
            self._require_persisted_payload_evidence(
                "link "
                f"{link.get('parent_action_id')}->{link.get('child_action_id')}",
                link.get("evidence"),
                payload_ids,
            )

    @staticmethod
    def _require_persisted_payload_evidence(
        owner: str,
        evidence: Any,
        payload_ids: set[int],
    ) -> None:
        if not isinstance(evidence, list) or any(
            not isinstance(item, dict) for item in evidence
        ):
            raise AssertionError(f"{owner} evidence must be an array of objects")
        for item in evidence:
            if item.get("kind") != "payload_segment":
                continue
            segment_id = item.get("id")
            if type(segment_id) is not int or segment_id not in payload_ids:
                raise AssertionError(
                    f"{owner} references unpersisted payload segment "
                    f"{segment_id!r}: {item}"
                )

    def _linked_child(
        self,
        parent_id: str,
        role: str,
        expected_kind: str,
        actions_by_id: dict[str, dict[str, Any]],
        links: list[dict[str, Any]],
    ) -> dict[str, Any]:
        matches = [
            link
            for link in links
            if link.get("parent_action_id") == parent_id
            and link.get("role") == role
        ]
        if len(matches) != 1:
            raise AssertionError(
                f"{parent_id} must have exactly one {role} link; found {matches}"
            )
        self._require_valid_link(matches[0])
        child_id = matches[0].get("child_action_id")
        child = actions_by_id.get(child_id)
        if child is None or child.get("kind") != expected_kind:
            raise AssertionError(
                f"{parent_id} {role} child must be {expected_kind}; "
                f"found id={child_id!r} action={child}"
            )
        return child

    def _linked_parent(
        self,
        child_id: str,
        role: str,
        actions_by_id: dict[str, dict[str, Any]],
        links: list[dict[str, Any]],
    ) -> dict[str, Any]:
        matches = [
            link
            for link in links
            if link.get("child_action_id") == child_id
            and link.get("role") == role
        ]
        if len(matches) != 1:
            raise AssertionError(
                f"{child_id} must have exactly one {role} parent link; "
                f"found {matches}"
            )
        self._require_valid_link(matches[0])
        parent_id = matches[0].get("parent_action_id")
        parent = actions_by_id.get(parent_id)
        if parent is None:
            raise AssertionError(
                f"{child_id} {role} parent action is missing: {parent_id!r}"
            )
        return parent

    @staticmethod
    def _require_valid_link(link: dict[str, Any]) -> None:
        if link.get("valid") is not True or link.get("confidence") != "observed":
            raise AssertionError(
                f"MCP semantic link must be valid/observed: {link}"
            )

    def _read_json(self, command: list[Path | str]) -> dict[str, Any]:
        output = self._runtime.run_checked(command, echo=False).stdout
        try:
            document = json.loads(output)
        except json.JSONDecodeError as error:
            raise AssertionError(
                f"viewer did not return valid JSON: {output[-4000:]}"
            ) from error
        if not isinstance(document, dict):
            raise AssertionError("viewer JSON output must be an object")
        return document

    @staticmethod
    def _object_list(
        document: dict[str, Any],
        key: str,
    ) -> list[dict[str, Any]]:
        values = document.get(key)
        if not isinstance(values, list) or any(
            not isinstance(value, dict) for value in values
        ):
            raise AssertionError(f"viewer {key} must be an array of objects")
        return values

    @staticmethod
    def _index_actions(
        actions: list[dict[str, Any]],
    ) -> dict[str, dict[str, Any]]:
        indexed: dict[str, dict[str, Any]] = {}
        for action in actions:
            action_id = action.get("action_id")
            if not isinstance(action_id, str) or not action_id:
                raise AssertionError(f"semantic action has invalid id: {action}")
            if action_id in indexed:
                raise AssertionError(f"duplicate semantic action id: {action_id}")
            indexed[action_id] = action
        return indexed
