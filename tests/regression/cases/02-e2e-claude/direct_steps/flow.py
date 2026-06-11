"""Direct Claude Code E2E phase runner."""

from __future__ import annotations

from pathlib import Path

from e2e_steps.binaries import require_actrail_binaries, require_actrailweb_binary
from e2e_steps.checks import capture_stdout, run_step
from e2e_steps.loader import load_module
from evidence import evidence_summary_facts, expected_found_detail
from model import FAIL, PASS, CaseResult

from .interactive import finish_claude_interactive_capture
from .launch import run_claude_launch_step
from .reporting import claude_output_summary, claude_tls_detail


def run_direct_claude_case(env, result: CaseResult, workload: dict[str, str]) -> None:
    module = load_module(
        "regression_claude_code_payload_run_e2e",
        env.repo_root / "tests/payload/claude-code/run_e2e.py",
    )
    resolved_config = None
    daemon = None
    result.command = ["direct", "claude-code"]
    run_step(
        result,
        "root privileges",
        module.require_root,
        expected_found_detail("uid=0", ["uid=0"]),
        "Claude payload capture requires eBPF/seccomp privileges",
    )
    actraild, actrailctl, actrailviewer = require_actrail_binaries(result, module, env.bin_dir)
    actrailweb = require_actrailweb_binary(result, module, env.bin_dir)
    tls_runtime = run_step(
        result,
        "Claude TLS runtime",
        lambda: module.resolve_optional_claude_tls_runtime(workload),
        lambda tls: expected_found_detail(
            "TLS runtime discovery chooses the capture source",
            [claude_tls_detail(tls)],
        ),
        "TLS runtime discovery decides whether request/response payloads come from user-space TLS or socket fallback",
    )
    values = run_step(
        result,
        "default operator config",
        lambda: module.read_config(env.default_operator_config_path()),
        lambda values: expected_found_detail(
            "default operator config can be parsed",
            [
                f"keys={len(values)}",
                f"path={env.default_operator_config_path()}",
            ],
        ),
        "export paths and daemon settings are read from the static default config",
    )
    run_step(
        result,
        "clean previous state",
        lambda: module.clean_configured_paths(actrailctl, resolved_config),
        expected_found_detail("previous configured state is removed", ["clean complete"]),
        "previous sockets, pid files, storage, and exports were removed",
    )
    daemon = run_step(
        result,
        "actraild daemon",
        lambda: module.start_daemon(
            actraild,
            resolved_config,
            float(module.required(workload, "daemon_ready_timeout_seconds")),
        ),
        expected_found_detail("actraild reports daemon listening", ["daemon listening"]),
        "the capture daemon accepted the rendered config",
        progress=True,
    )
    try:
        finish_claude_capture(
            env,
            result,
            module,
            workload,
            values,
            resolved_config,
            daemon,
            actrailctl,
            actrailviewer,
            actrailweb,
            tls_runtime,
        )
        finish_claude_interactive_capture(
            result,
            module,
            workload,
            resolved_config,
            daemon,
            actrailctl,
            actrailviewer,
            tls_runtime,
        )
    finally:
        if daemon is not None:
            module.stop_process(
                daemon,
                float(module.required(workload, "daemon_stop_timeout_seconds")),
            )


def finish_claude_capture(
    env,
    result: CaseResult,
    module,
    workload: dict[str, str],
    values: dict[str, str],
    resolved_config: Path | None,
    daemon,
    actrailctl: Path,
    actrailviewer: Path,
    actrailweb: Path,
    tls_runtime,
) -> None:
    trace_id = run_claude_launch_step(result, module, daemon, actrailctl, resolved_config, workload)
    payloads = run_step(
        result,
        "payload rows",
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
            "viewer returns accepted payload rows",
            [
                f"trace_id=trace-{trace_id}",
                f"payload_output_bytes={len(rows.encode('utf-8'))}",
            ],
        ),
        "payload table contains complete Claude request rows and TLS response rows when TLS capture is enabled",
        progress=True,
    )
    payload_count = run_step(
        result,
        "payload capture",
        lambda: module.require_complete_payload_rows_any(
            payloads,
            module.accepted_payload_sources(tls_runtime),
            direction="outbound",
        ),
        lambda count: expected_found_detail("complete outbound payload segments exist", [f"captured_payload_segments={count}"]),
        "viewer observed complete outbound payload segments for the Claude request",
    )
    response_count = run_step(
        result,
        "response payload gate",
        lambda: module.require_tls_response_payloads(payloads, tls_runtime),
        lambda count: expected_found_detail(
            "inbound TLS response rows are required only when this trace has outbound TLS rows",
            module.tls_response_evidence_facts(
                payloads,
                module.accepted_tls_payload_sources(tls_runtime),
                count,
            ),
        ),
        "HTTP/socket provider routes must not be blocked by a separately discoverable Claude TLS runtime",
    )
    actions = run_step(
        result,
        "semantic actions",
        lambda: module.wait_for_semantic_actions(
            actrailviewer,
            resolved_config,
            trace_id,
            int(module.required(workload, "drain_attempts")),
            float(module.required(workload, "drain_sleep_seconds")),
        ),
        lambda rows: expected_found_detail(
            "viewer returns semantic llm.request and llm.response actions",
            [
                f"trace_id=trace-{trace_id}",
                f"action_output_bytes={len(rows.encode('utf-8'))}",
            ],
        ),
        "semantic action projection ran after payload ingestion",
        progress=True,
    )
    run_step(
        result,
        "complete LLM exchange actions",
        lambda: module.require_llm_exchange(actions),
        expected_found_detail(
            "complete successful llm.call/request/response exist",
            ["complete successful llm.call", "complete successful llm.request", "complete successful llm.response"],
        ),
        "the action table contains a complete successful semantic request/response exchange",
    )
    run_step(
        result,
        "LLM exchange action graph",
        lambda: module.require_llm_exchange_graph(actions),
        expected_found_detail(
            "llm.call links to request/response and HTTP evidence",
            ["llm.call.request", "llm.call.response", "llm.request.http_message", "llm.response facts"],
        ),
        "viewer JSON exposes the semantic action graph without direct SQLite inspection",
    )
    run_step(
        result,
        "Web action-tree reachability",
        lambda: module.require_web_action_tree_projection(
            actrailweb,
            resolved_config,
            trace_id,
            float(module.required(workload, "daemon_ready_timeout_seconds")),
            float(module.required(workload, "drain_sleep_seconds")),
            required_reachable_kinds=("llm.call", "llm.request", "llm.response", "http.message"),
        ),
        lambda summary: expected_found_detail(
            "web action-tree recursively reaches every display action",
            [
                f"actions={summary['action_count']}",
                f"reachable={summary['reachable_count']}",
                f"http_messages={summary['kind_counts'].get('http.message', 0)}",
            ],
        ),
        "actrailweb action-tree API exposes the same semantic actions that the viewer generated",
    )
    text = run_step(
        result,
        "payload text fetch",
        lambda: module.payload_texts(
            actrailviewer,
            resolved_config,
            trace_id,
            payloads,
            int(module.required(workload, "payload_fetch_count")),
        ),
        lambda text: expected_found_detail("payload text can be fetched", [f"captured_payload_text_bytes={len(text.encode('utf-8'))}"]),
        "payload text can be fetched from captured segment ids",
    )
    run_step(
        result,
        "non-empty payload text",
        lambda: module.require_non_empty_payload_text(text),
        expected_found_detail("payload text is non-empty", ["payload text is non-empty"]),
        "captured payload text is available for marker/export validation",
    )
    marker = module.required(workload, "prompt_marker")
    export_document = run_step(
        result,
        "JSON export",
        lambda: module.export_trace_json(actrailviewer, values, resolved_config, trace_id),
        lambda document: expected_found_detail("trace graph JSON contains payload nodes", [f"exported_payload_nodes={module.count_payload_nodes(document)}"]),
        "actrailviewer exported the trace graph JSON",
    )
    run_step(
        result,
        "exported payload marker",
        lambda: module.require_exported_payload_marker(export_document, marker),
        expected_found_detail("JSON graph export contains request marker", [f"exported_payload_marker={marker}"]),
        "JSON graph export includes a Payload node with the request marker",
    )
    otel_document = run_step(
        result,
        "OTEL export",
        lambda: module.export_trace_otel(actrailviewer, values, resolved_config, trace_id),
        lambda document: expected_found_detail("semantic OTEL spans are exported", [f"otel_semantic_spans={module.count_otel_spans(document)}"]),
        "actrailviewer exported semantic OTEL JSON",
    )
    run_step(
        result,
        "OTEL payload marker",
        lambda: module.require_exported_llm_span(otel_document, marker),
        expected_found_detail("OTEL llm.request payload contains request marker", [f"otel_payload_marker={marker}"]),
        "the llm.request payload text exported to OTEL contains the prompt marker",
    )
    run_step(
        result,
        "llm.response OTEL span",
        lambda: module.require_exported_llm_response_span(otel_document),
        expected_found_detail(
            "OTEL contains a complete successful llm.response span",
            ["claude_llm_response_span=present"],
        ),
        "OTEL export contains the captured provider response as a semantic span",
    )
    _, evidence_text = capture_stdout(
        lambda: module.emit_llm_otel_evidence(
            otel_document,
            int(module.required(workload, "evidence_text_max_chars")),
        )
    )
    result.stdout_tail = env.output_tail(
        claude_output_summary(
            trace_id,
            payload_count,
            response_count,
            text,
            export_document,
            actions,
            otel_document,
            marker,
            evidence_text,
            module,
        )
    )
    result.add_check(
        "LLM exchange content",
        PASS
        if (
            "evidence.llm_request.body_text_json=" in evidence_text
            and "evidence.llm_response.body_text_json=" in evidence_text
        )
        else FAIL,
        expected_found_detail(
            "OTEL evidence includes request and response payload summaries",
            evidence_summary_facts(
                evidence_text,
                (
                    "evidence.llm_request.model=",
                    "evidence.llm_request.route=",
                    "evidence.llm_request.source=",
                    "evidence.llm_request.body_text_json=",
                    "evidence.llm_response.model=",
                    "evidence.llm_response.source=",
                    "evidence.llm_response.payload_bytes=",
                    "evidence.llm_response.body_text_json=",
                ),
            ),
        ),
        "OTEL evidence must include parsed llm.request content and captured llm.response content",
    )
