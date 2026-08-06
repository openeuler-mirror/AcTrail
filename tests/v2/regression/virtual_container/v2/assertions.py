from __future__ import annotations

import json
import re
from dataclasses import dataclass
from typing import Any


@dataclass(frozen=True)
class TraceView:
    trace_id: str
    name: str
    state: str
    health: str


@dataclass(frozen=True)
class SummaryCounts:
    events: int
    network_events: int


def find_clean_trace(viewer_json: str, title: str) -> TraceView:
    try:
        document = json.loads(viewer_json)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"viewer returned invalid trace JSON: {error}") from error
    traces = _object(document, "viewer output").get("traces")
    if not isinstance(traces, list):
        raise RuntimeError("viewer trace JSON must contain a traces array")
    matches = [
        _trace_view(item)
        for item in traces
        if isinstance(item, dict) and item.get("name") == title
    ]
    if not matches:
        raise RuntimeError(f"viewer returned no trace with title: {title}")
    if len(matches) != 1:
        raise RuntimeError(f"viewer returned duplicate traces with title: {title}")
    trace = matches[0]
    if trace.state not in {"Completed", "Exited"}:
        raise RuntimeError(
            f"trace {trace.trace_id} is not terminal-clean: state={trace.state}"
        )
    if trace.health != "Clean":
        raise RuntimeError(
            f"trace {trace.trace_id} is not healthy: health={trace.health}"
        )
    return trace


def parse_summary_counts(summary: str) -> SummaryCounts:
    events = _integer_field(summary, "events")
    network_events = _integer_field(summary, "network_events")
    return SummaryCounts(events=events, network_events=network_events)


def require_markers(output: str, markers: tuple[str, ...], *, context: str) -> None:
    missing = [marker for marker in markers if marker not in output]
    if missing:
        raise RuntimeError(
            f"{context} omitted required marker(s): " + ", ".join(missing)
        )


def reject_markers(output: str, markers: tuple[str, ...], *, context: str) -> None:
    present = [marker for marker in markers if marker in output]
    if present:
        raise RuntimeError(
            f"{context} contained cross-trace marker(s): " + ", ".join(present)
        )


def _trace_view(value: dict[str, Any]) -> TraceView:
    fields = {}
    for name in ("trace_id", "name", "state", "health"):
        field = value.get(name)
        if not isinstance(field, str) or not field:
            raise RuntimeError(f"viewer trace JSON has invalid {name}")
        fields[name] = field
    return TraceView(**fields)


def _object(value: Any, name: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise RuntimeError(f"{name} must be a JSON object")
    return value


def _integer_field(output: str, name: str) -> int:
    match = re.search(rf"(?:^|\s){re.escape(name)}=([0-9]+)(?:\s|$)", output)
    if match is None:
        raise RuntimeError(f"trace summary omitted numeric {name}")
    return int(match.group(1))
