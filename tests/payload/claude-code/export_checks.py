"""Claude Code E2E export and semantic action checks."""

from __future__ import annotations

import json
from pathlib import Path

from common import (
    actrail_command,
    count_action_rows,
    require_complete_llm_exchange,
    require_llm_exchange_graph,
    required,
    run_checked,
    wait_for_llm_exchange_actions,
)


def export_trace_json(
    actrailviewer: Path,
    values: dict[str, str],
    config: Path | None,
    trace_id: int,
) -> dict:
    output_path = Path(required(values, "export_directory")) / f"trace-{trace_id}-payload.json"
    run_checked(
        actrail_command(
            actrailviewer,
            config,
            "export-json",
            "--trace-id",
            str(trace_id),
            "--output",
            str(output_path),
        ),
        echo=False,
    )
    with output_path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def export_trace_otel(
    actrailviewer: Path,
    values: dict[str, str],
    config: Path | None,
    trace_id: int,
) -> dict:
    output_path = Path(required(values, "export_directory")) / f"trace-{trace_id}-semantic.otlp.json"
    run_checked(
        actrail_command(
            actrailviewer,
            config,
            "export-otel",
            "--trace-id",
            str(trace_id),
            "--output",
            str(output_path),
        ),
        echo=False,
    )
    with output_path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def require_exported_payload_marker(document: dict, marker: str) -> None:
    for node in payload_nodes(document):
        attributes = node.get("attributes", {})
        if (
            attributes.get("direction") == "Outbound"
            and attributes.get("bytes_base64")
            and marker in attributes.get("text", "")
        ):
            return
    raise RuntimeError(f"exported JSON payload nodes did not contain prompt marker {marker}")


def wait_for_semantic_actions(
    actrailviewer: Path,
    config: Path | None,
    trace_id: int,
    attempts: int,
    sleep_sec: float,
) -> str:
    return wait_for_llm_exchange_actions(
        actrailviewer,
        config,
        trace_id,
        attempts,
        sleep_sec,
    )


def require_llm_exchange(actions: str) -> None:
    require_complete_llm_exchange(actions)


def require_exported_llm_span(document: dict, marker: str) -> None:
    for span in otel_spans(document):
        attributes = otel_attributes(span)
        body_text = attributes.get("http.request.body_text", "")
        if (
            attributes.get("actrail.action.kind") == "llm.request"
            and attributes.get("actrail.action.completeness") == "complete"
            and attributes.get("actrail.action.status") == "success"
            and attributes.get("llm.request.payload_text")
            and attributes.get("llm.request.payload_bytes")
            and attributes.get("llm.request.raw_payload_bytes")
            and attributes.get("payload.source_boundary") in {"TlsUserSpace", "Syscall"}
            and attributes.get("url.scheme") in {"http", "https"}
            and body_text.lstrip().startswith("{")
            and '"model"' in body_text
            and marker in attributes.get("llm.request.payload_text", "")
            and (
                attributes.get("http.request.headers_hpack_base64")
                or attributes.get("http.request.headers_text")
            )
        ):
            return
    raise RuntimeError(
        "OTel export did not contain a complete llm.request span with split HTTP headers/body and prompt marker"
    )


def require_exported_llm_response_span(document: dict) -> None:
    for span in otel_spans(document):
        attributes = otel_attributes(span)
        if (
            attributes.get("actrail.action.kind") == "llm.response"
            and attributes.get("actrail.action.completeness") == "complete"
            and attributes.get("actrail.action.status") == "success"
            and attributes.get("llm.response.payload_text")
            and attributes.get("llm.response.payload_bytes")
            and attributes.get("payload.source_boundary") in {"TlsUserSpace", "Syscall"}
        ):
            return
    raise RuntimeError("OTel export did not contain a complete llm.response span")


def count_otel_spans(document: dict) -> int:
    return len(otel_spans(document))


def otel_spans(document: dict) -> list[dict]:
    spans: list[dict] = []
    for resource_span in document.get("resourceSpans", []):
        for scope_span in resource_span.get("scopeSpans", []):
            spans.extend(scope_span.get("spans", []))
    return spans


def otel_attributes(span: dict) -> dict[str, str]:
    output: dict[str, str] = {}
    for attribute in span.get("attributes", []):
        value = attribute.get("value", {})
        if "stringValue" in value:
            output[attribute.get("key", "")] = value["stringValue"]
        elif "intValue" in value:
            output[attribute.get("key", "")] = str(value["intValue"])
    return output


def count_payload_nodes(document: dict) -> int:
    return len(payload_nodes(document))


def payload_nodes(document: dict) -> list[dict]:
    return [node for node in document.get("nodes", []) if node.get("kind") == "Payload"]
