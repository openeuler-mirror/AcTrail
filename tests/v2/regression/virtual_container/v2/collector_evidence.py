"""Host Collector evidence for the virtual-container OTLP egress path.

The Guest-local SQLite assertions cannot tell whether a span ever left the
sandbox. This module reads the Host Collector's JSON file exporter output, so
the acceptance run proves delivery instead of relying on a manual log check.
"""

from __future__ import annotations

import json
import time
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class CollectorEvidence:
    spans: int
    traces: int
    unreadable_lines: int


def read_spans_since(path: Path, since_unix_nano: int) -> CollectorEvidence:
    """Count spans started at or after ``since_unix_nano`` in a JSON-lines file."""

    spans = 0
    trace_ids: set[str] = set()
    unreadable_lines = 0
    try:
        content = path.read_text(encoding="utf-8", errors="replace")
    except FileNotFoundError:
        return CollectorEvidence(0, 0, 0)
    for line in content.splitlines():
        if not line.strip():
            continue
        try:
            document = json.loads(line)
        except json.JSONDecodeError:
            # The exporter may be in the middle of appending the last batch.
            unreadable_lines += 1
            continue
        for span in _spans(document):
            try:
                started = int(span.get("startTimeUnixNano", 0))
            except (TypeError, ValueError):
                continue
            if started < since_unix_nano:
                continue
            spans += 1
            trace_id = span.get("traceId")
            if isinstance(trace_id, str) and trace_id:
                trace_ids.add(trace_id)
    return CollectorEvidence(spans, len(trace_ids), unreadable_lines)


def wait_for_spans(
    path: Path,
    since_unix_nano: int,
    timeout_seconds: float,
    *,
    clock: Callable[[], float] = time.monotonic,
    sleep: Callable[[float], None] = time.sleep,
) -> CollectorEvidence:
    """Wait until the Collector has written at least one span of this run."""

    deadline = clock() + timeout_seconds
    while True:
        evidence = read_spans_since(path, since_unix_nano)
        if evidence.spans > 0:
            return evidence
        if clock() >= deadline:
            state = "exists" if path.exists() else "does not exist"
            raise RuntimeError(
                f"no span from this run reached the Host Collector within "
                f"{timeout_seconds:g}s: {path} {state}, "
                f"unreadable_lines={evidence.unreadable_lines}"
            )
        sleep(0.5)


def _spans(document: object):
    if not isinstance(document, dict):
        return
    for resource_spans in _list(document.get("resourceSpans")):
        if not isinstance(resource_spans, dict):
            continue
        for scope_spans in _list(resource_spans.get("scopeSpans")):
            if not isinstance(scope_spans, dict):
                continue
            for span in _list(scope_spans.get("spans")):
                if isinstance(span, dict):
                    yield span


def _list(value: object) -> list:
    return value if isinstance(value, list) else []
