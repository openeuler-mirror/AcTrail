"""Interactive-entry Claude capture checks."""

from __future__ import annotations

from pathlib import Path

from e2e_steps.checks import run_step
from evidence import expected_found_detail
from model import CaseResult

from .launch import run_claude_interactive_launch_step


def finish_claude_interactive_capture(
    result: CaseResult,
    module,
    workload: dict[str, str],
    resolved_config: Path,
    daemon,
    actrailctl: Path,
    actrailviewer: Path,
    tls_runtime,
) -> None:
    trace_id = run_claude_interactive_launch_step(
        result,
        module,
        daemon,
        actrailctl,
        resolved_config,
        workload,
    )
    payloads = run_step(
        result,
        "interactive payload rows",
        lambda: module.wait_for_llm_payloads(
            actrailctl,
            actrailviewer,
            resolved_config,
            trace_id,
            int(module.required(workload, "drain_attempts")),
            float(module.required(workload, "drain_sleep_seconds")),
            module.required(workload, "payload_head"),
            module.accepted_payload_sources(tls_runtime),
            module.accepted_tls_payload_sources(tls_runtime),
        ),
        lambda rows: expected_found_detail(
            "viewer returns accepted payload rows for interactive Claude",
            [
                f"trace_id=trace-{trace_id}",
                f"payload_output_bytes={len(rows.encode('utf-8'))}",
            ],
        ),
        "payload table contains complete request rows for `claude <prompt>`",
        progress=True,
    )
    run_step(
        result,
        "interactive payload capture",
        lambda: module.require_complete_payload_rows_any(
            payloads,
            module.accepted_payload_sources(tls_runtime),
            direction="outbound",
        ),
        lambda count: expected_found_detail(
            "complete outbound payload segments exist for interactive Claude",
            [f"interactive_payload_segments={count}"],
        ),
        "viewer observed complete outbound payload segments for `claude <prompt>`",
    )
    run_step(
        result,
        "interactive response payload gate",
        lambda: module.require_tls_response_payloads(payloads, tls_runtime),
        lambda count: expected_found_detail(
            "inbound TLS response rows are accounted for interactive Claude",
            [f"interactive_response_payload_segments={count}"],
        ),
        "TLS response payload gate also holds for `claude <prompt>`",
    )
    actions = run_step(
        result,
        "interactive semantic actions",
        lambda: module.wait_for_semantic_actions(
            actrailviewer,
            resolved_config,
            trace_id,
            int(module.required(workload, "drain_attempts")),
            float(module.required(workload, "drain_sleep_seconds")),
        ),
        lambda rows: expected_found_detail(
            "viewer returns interactive llm.request and llm.response actions",
            [
                f"trace_id=trace-{trace_id}",
                f"action_output_bytes={len(rows.encode('utf-8'))}",
            ],
        ),
        "semantic action projection ran for `claude <prompt>`",
        progress=True,
    )
    run_step(
        result,
        "interactive complete LLM exchange actions",
        lambda: module.require_llm_exchange(actions),
        expected_found_detail(
            "complete successful interactive llm.call/request/response exist",
            ["complete successful llm.call", "complete successful llm.request", "complete successful llm.response"],
        ),
        "`claude <prompt>` produced a complete semantic request/response exchange",
    )
    run_step(
        result,
        "interactive LLM exchange action graph",
        lambda: module.require_llm_exchange_graph(actions),
        expected_found_detail(
            "interactive llm.call links to request/response and HTTP evidence",
            ["llm.call.request", "llm.call.response", "llm.request.http_message", "llm.response facts"],
        ),
        "viewer JSON exposes the interactive semantic action graph",
    )
