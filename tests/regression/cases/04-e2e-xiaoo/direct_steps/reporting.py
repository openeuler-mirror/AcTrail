"""xiaoO evidence formatting helpers."""

from __future__ import annotations


def xiaoo_tls_detail(tls_runtime) -> str:
    return f"xiaoo_tls_runtime={tls_runtime.detail}"


def xiaoo_output_summary(
    trace_id: int,
    payload_count: int,
    request_span_count: int,
    response_span_count: int,
    launch_output: str,
    evidence_text: str,
) -> str:
    return (
        f"xiaoo_trace_id={trace_id}\n"
        f"xiaoo_payload_segments={payload_count}\n"
        f"xiaoo_llm_request_spans={request_span_count}\n"
        f"xiaoo_llm_response_spans={response_span_count}\n"
        f"{launch_output}\n"
        f"{evidence_text}"
    )
