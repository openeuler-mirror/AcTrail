"""opencode evidence formatting helpers."""

from __future__ import annotations


def opencode_tls_detail(tls_runtime) -> str:
    if tls_runtime is None:
        return "opencode_tls_runtime=disabled"
    return f"opencode_tls_runtime=auto provider={tls_runtime.provider} detail={tls_runtime.detail}"


def opencode_output_summary(
    trace_id: int,
    payload_count: int,
    response_count: int,
    request_span_count: int,
    response_span_count: int,
    launch_output: str,
    evidence_text: str,
) -> str:
    return (
        f"opencode_trace_id={trace_id}\n"
        f"opencode_payload_segments={payload_count}\n"
        f"opencode_response_payload_segments={response_count}\n"
        f"opencode_llm_request_spans={request_span_count}\n"
        f"opencode_llm_response_spans={response_span_count}\n"
        f"{launch_output}\n"
        f"{evidence_text}"
    )
