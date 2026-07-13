"""Structured semantic-action snapshot used by process E2E tests."""

from __future__ import annotations

import json
import subprocess
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class ActionRecord:
    action_id: str
    kind: str
    process_id: int
    attributes: dict[str, str]


@dataclass(frozen=True)
class ActionLinkRecord:
    parent_action_id: str
    child_action_id: str
    role: str
    valid: bool


class SemanticActionSnapshot:
    def __init__(
        self,
        actions: tuple[ActionRecord, ...],
        links: tuple[ActionLinkRecord, ...],
    ) -> None:
        self._actions = actions
        self._links = links
        self._actions_by_id = {action.action_id: action for action in actions}
        if len(self._actions_by_id) != len(actions):
            raise RuntimeError("semantic action snapshot contains duplicate action IDs")

    @classmethod
    def load(cls, viewer: Path, config: Path, trace_id: int) -> SemanticActionSnapshot:
        command = [
            str(viewer),
            "--config",
            str(config),
            "--output-format",
            "json",
            "actions",
            "--trace-id",
            str(trace_id),
        ]
        result = subprocess.run(command, text=True, capture_output=True, check=False)
        if result.returncode != 0:
            raise RuntimeError(
                "semantic action snapshot failed: "
                f"command={' '.join(command)} stdout={result.stdout} stderr={result.stderr}"
            )
        document = json.loads(result.stdout)
        actions = tuple(cls._action(row) for row in document.get("actions", []))
        links = tuple(cls._link(row) for row in document.get("links", []))
        return cls(actions, links)

    def actions(self, kind: str) -> tuple[ActionRecord, ...]:
        return tuple(action for action in self._actions if action.kind == kind)

    def valid_linked_children(
        self,
        role: str,
        child_kind: str,
    ) -> tuple[ActionRecord, ...]:
        children = []
        for link in self._links:
            if not link.valid or link.role != role:
                continue
            child = self._actions_by_id.get(link.child_action_id)
            if child is not None and child.kind == child_kind:
                children.append(child)
        return tuple(children)

    @staticmethod
    def _action(row: dict) -> ActionRecord:
        process = row.get("process")
        if not isinstance(process, dict) or "process_id" not in process:
            raise RuntimeError(f"semantic action has no logical process identity: {row}")
        return ActionRecord(
            action_id=str(row["action_id"]),
            kind=str(row["kind"]),
            process_id=int(process["process_id"]),
            attributes={str(key): str(value) for key, value in row.get("attributes", {}).items()},
        )

    @staticmethod
    def _link(row: dict) -> ActionLinkRecord:
        return ActionLinkRecord(
            parent_action_id=str(row["parent_action_id"]),
            child_action_id=str(row["child_action_id"]),
            role=str(row["role"]),
            valid=bool(row.get("valid", False)),
        )
