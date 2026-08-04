from __future__ import annotations

import json
from typing import Any

from .probe import McpProbeSpec


class McpActionContract:
    def require_identity(
        self,
        action: dict[str, Any],
        expected: McpProbeSpec,
    ) -> None:
        self.require_attributes(
            action,
            {
                "mcp.server.name": expected.server_name,
                "mcp.tool.name": expected.tool_name,
                "mcp.tool.id": expected.tool_name,
                "mcp.transport": "stdio",
            },
        )

    def require_terminal(self, action: dict[str, Any]) -> None:
        if (
            action.get("status") != "success"
            or action.get("completeness") != "complete"
        ):
            raise AssertionError(
                f"{action.get('action_id')} ({action.get('kind')}) must be "
                f"success/complete; status={action.get('status')} "
                f"completeness={action.get('completeness')}"
            )
        if action.get("end_time_unix_nanos") is None:
            raise AssertionError(
                f"{action.get('action_id')} ({action.get('kind')}) has no end time"
            )

    def require_attributes(
        self,
        action: dict[str, Any],
        expected: dict[str, Any],
    ) -> None:
        attributes = self.attributes(action)
        mismatches = {
            key: {"expected": value, "actual": attributes.get(key)}
            for key, value in expected.items()
            if attributes.get(key) != value
        }
        if mismatches:
            raise AssertionError(
                f"{action.get('action_id')} attribute mismatch: {mismatches}"
            )

    @staticmethod
    def require_reference(
        attributes: dict[str, Any],
        key: str,
        action: dict[str, Any],
    ) -> None:
        if attributes.get(key) != action.get("action_id"):
            raise AssertionError(
                f"{key} must reference {action.get('action_id')}; "
                f"found {attributes.get(key)!r}"
            )

    @staticmethod
    def attributes(action: dict[str, Any]) -> dict[str, Any]:
        attributes = action.get("attributes")
        if not isinstance(attributes, dict):
            raise AssertionError(
                f"{action.get('action_id')} attributes must be an object"
            )
        return attributes

    def root_identity(
        self,
        action: dict[str, Any],
    ) -> tuple[Any, Any, Any]:
        attributes = self.attributes(action)
        return (
            attributes.get("mcp.server.name"),
            attributes.get("mcp.tool.name"),
            attributes.get("mcp.transport"),
        )

    @staticmethod
    def report(actions: list[dict[str, Any]]) -> str:
        return json.dumps(
            [
                {
                    "action_id": action.get("action_id"),
                    "kind": action.get("kind"),
                    "status": action.get("status"),
                    "completeness": action.get("completeness"),
                    "attributes": action.get("attributes"),
                }
                for action in actions
            ],
            sort_keys=True,
        )
