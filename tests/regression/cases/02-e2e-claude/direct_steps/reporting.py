"""Claude evidence formatting helpers."""

from __future__ import annotations


def claude_tls_detail(tls_runtime) -> str:
    if tls_runtime is None:
        return "claude_tls_runtime=disabled"
    return f"claude_tls_runtime={tls_runtime.detail}"


def claude_output_summary(
    trace_id: int,
    payload_count: int,
    response_count: int,
    text: str,
    export_document: dict,
    actions: str,
    otel_document: dict,
    marker: str,
    evidence_text: str,
    module,
) -> str:
    return (
        f"claude_code_trace_id={trace_id}\n"
        f"captured_payload_segments={payload_count}\n"
        f"captured_response_payload_segments={response_count}\n"
        f"captured_payload_text_bytes={len(text.encode('utf-8'))}\n"
        f"exported_payload_nodes={module.count_payload_nodes(export_document)}\n"
        f"exported_payload_marker={marker}\n"
        f"semantic_action_rows={module.count_action_rows(actions)}\n"
        f"otel_semantic_spans={module.count_otel_spans(otel_document)}\n"
        f"otel_payload_marker={marker}\n"
        f"{evidence_text}"
    )
