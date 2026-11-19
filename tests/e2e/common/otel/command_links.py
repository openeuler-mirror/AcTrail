from __future__ import annotations

from collections import Counter
from collections.abc import Callable
from typing import Any


class CommandLinkVerification:
    """Compare real received command topology with persisted relations."""

    _PARENT_ROLES = ("command.contains_command_invocation", "agent.performed_action")
    _CONFLICT_ROLES = {*_PARENT_ROLES, "command.contains_mcp_tool_call"}

    def __init__(
        self,
        get: Callable[[str], dict[str, Any]],
        *,
        allow_late_agent_links: bool = False,
    ):
        self._get = get
        self._allow_late_agent_links = allow_late_agent_links

    def verify(
        self,
        trace_id: int,
        actions: list[dict[str, Any]],
        spans: dict[str, dict[str, Any]],
    ) -> None:
        timeline = self._get(
            f"/api/traces/{trace_id}/actions/timeline?kinds=command.invocation&limit=1000"
        )
        if timeline.get("next_cursor") or timeline.get("links_truncated") is not False:
            raise AssertionError("command topology verification requires a complete timeline")
        local_ids = {action["id"] for action in actions}
        if not local_ids.issubset({action["id"] for action in timeline["actions"]}):
            raise AssertionError("timeline did not include all local commands")
        linked = 0
        for action in actions:
            span = spans[action["id"]]
            incoming = [
                link for link in timeline["links"]
                if link["child"] == action["id"] and link["valid"]
            ]
            if action["attributes"].get("process.parent.identity_state") == "conflict":
                incoming = [link for link in incoming if link["role"] not in self._CONFLICT_ROLES]
            if span["spanId"] != self._span_id(action["id"]):
                raise AssertionError(f"{action['id']} Span ID differs from local action identity")
            received_links = Counter(
                (link["traceId"], link["spanId"],
                 self._attributes(link).get("actrail.link.role"),
                 self._attributes(link).get("actrail.link.origin"))
                for link in span.get("links", [])
            )
            allowed_links = Counter(
                (span["traceId"], self._span_id(link["parent"]), link["role"], link["origin"])
                for link in incoming
            )
            # JSONL wrapper commands may finish before Agent identity is established.
            required = [
                link for link in incoming
                if not self._allow_late_agent_links or link["role"] != "agent.performed_action"
            ]
            required_links = Counter(
                (span["traceId"], self._span_id(link["parent"]), link["role"], link["origin"])
                for link in required
            )
            if received_links - allowed_links or required_links - received_links:
                raise AssertionError(
                    f"{action['id']} Span links differ from established relations: "
                    f"actual={received_links}, required={required_links}, allowed={allowed_links}"
                )
            parent_links = [
                link for link in incoming
                if link in required or received_links[
                    (span["traceId"], self._span_id(link["parent"]), link["role"], link["origin"])
                ]
            ]
            expected_parents: set[str] = set()
            for role in self._PARENT_ROLES:
                expected_parents = {
                    self._span_id(link["parent"]) for link in parent_links if link["role"] == role
                }
                if expected_parents:
                    break
            if expected_parents:
                linked += 1
                if span.get("parentSpanId") not in expected_parents:
                    raise AssertionError(
                        f"{action['id']} parentSpanId differs from established parents: "
                        f"actual={span.get('parentSpanId')}, expected={sorted(expected_parents)}"
                    )
            elif span.get("parentSpanId"):
                raise AssertionError(f"{action['id']} Span has an unexpected parent")
        if linked == 0:
            raise AssertionError("real commands produced no parent relations to verify")

    @staticmethod
    def _span_id(action_id: str) -> str:
        value = 0xCBF29CE484222325
        for byte in action_id.encode("utf-8"):
            value = ((value ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
        return f"{value:016x}"

    @staticmethod
    def _attributes(value: dict[str, Any]) -> dict[str, str]:
        return {
            attribute["key"]: str(attribute["value"].get(
                "stringValue", attribute["value"].get("intValue", "")
            ))
            for attribute in value.get("attributes", [])
        }
