"""OTEL export and evidence helpers."""

from __future__ import annotations

import json
from pathlib import Path

from .config import actrail_command, run_checked


def export_otel(actrailviewer: Path, config: Path | None, trace_id: int, output: Path) -> dict:
    if output.exists():
        output.unlink()
    run_checked(
        actrail_command(
            actrailviewer,
            config,
            "export-otel",
            "--trace-id",
            str(trace_id),
            "--output",
            str(output),
        ),
        echo=False,
    )
    return json.loads(output.read_text(encoding="utf-8"))


def require_otel_span(document: dict, kind: str) -> int:
    count = 0
    for span in otel_spans(document):
        attrs = otel_attrs(span)
        if attrs.get("actrail.action.kind") == kind:
            count += 1
    if count == 0:
        raise RuntimeError(f"OTEL export did not contain {kind} span")
    return count


def emit_llm_otel_evidence(document: dict, max_text_chars: int) -> None:
    request = first_otel_action(document, "llm.request")
    if request is None:
        print("evidence.llm_request=not exported")
    else:
        attrs = otel_attrs(request)
        route = attrs.get("url.path", "")
        method = attrs.get("http.request.method", "")
        scheme = attrs.get("url.scheme", "")
        body = attrs.get("http.request.body_text") or attrs.get("llm.request.payload_text", "")
        print(f"evidence.llm_request.model={attrs.get('llm.request.model', '')}")
        print(f"evidence.llm_request.source={attrs.get('payload.source_boundary', '')}")
        print(f"evidence.llm_request.route={scheme} {method} {route}".rstrip())
        print(f"evidence.llm_request.payload_bytes={attrs.get('llm.request.payload_bytes', '')}")
        print(
            "evidence.llm_request.body_text_json="
            f"{json.dumps(clip_text(body, max_text_chars), ensure_ascii=False)}"
        )

    response = first_otel_action(document, "llm.response")
    if response is None:
        print("evidence.llm_response=not exported")
    else:
        attrs = otel_attrs(response)
        body = attrs.get("http.response.body_text") or attrs.get("llm.response.payload_text", "")
        print(f"evidence.llm_response.model={attrs.get('llm.response.model', '')}")
        print(f"evidence.llm_response.source={attrs.get('payload.source_boundary', '')}")
        print(f"evidence.llm_response.payload_bytes={attrs.get('llm.response.payload_bytes', '')}")
        print(
            "evidence.llm_response.body_text_json="
            f"{json.dumps(clip_text(body, max_text_chars), ensure_ascii=False)}"
        )


def first_otel_action(document: dict, kind: str) -> dict | None:
    for span in otel_spans(document):
        attrs = otel_attrs(span)
        if attrs.get("actrail.action.kind") == kind:
            return span
    return None


def clip_text(text: str, max_chars: int) -> str:
    if max_chars < 0:
        raise RuntimeError("evidence text max chars must be non-negative")
    return text if len(text) <= max_chars else text[:max_chars] + "...[truncated]"


def otel_spans(document: dict) -> list[dict]:
    spans: list[dict] = []
    for resource in document.get("resourceSpans", []):
        for scope in resource.get("scopeSpans", []):
            spans.extend(scope.get("spans", []))
    return spans


def otel_attrs(span: dict) -> dict[str, str]:
    attrs: dict[str, str] = {}
    for attr in span.get("attributes", []):
        value = attr.get("value", {})
        if "stringValue" in value:
            attrs[attr.get("key", "")] = value["stringValue"]
        elif "intValue" in value:
            attrs[attr.get("key", "")] = str(value["intValue"])
    return attrs
