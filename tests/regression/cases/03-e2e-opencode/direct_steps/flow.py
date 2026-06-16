"""Direct opencode E2E phase runner."""

from __future__ import annotations

import os
from pathlib import Path

from e2e_steps.binaries import require_actrail_binaries, require_actrailweb_binary
from e2e_steps.checks import capture_stdout, run_step
from e2e_steps.loader import load_module
from evidence import evidence_summary_facts, expected_found_detail
from model import FAIL, PASS, CaseResult

from .launch import run_opencode_launch_step
from .reporting import opencode_output_summary, opencode_tls_detail


def run_direct_opencode_case(
    env,
    result: CaseResult,
    entry: Path,
) -> None:
    module = load_module(
        "regression_opencode_bun_run_e2e",
        env.repo_root / "tests/agent-trace/opencode-bun/run_e2e.py",
    )
    old_path = os.environ.get("PATH", "")
    os.environ["PATH"] = f"{entry.parent}{os.pathsep}{old_path}"
    try:
        run_loaded_opencode_case(env, result, module)
    finally:
        os.environ["PATH"] = old_path


def run_loaded_opencode_case(env, result: CaseResult, module) -> None:
    case_dir = env.repo_root / "tests/agent-trace/opencode-bun"
    daemon = None
    result.command = ["direct", "opencode-bun"]
    workload = run_step(
        result,
        "opencode workload config",
        lambda: module.read_config(case_dir / "workload.conf"),
        lambda values: expected_found_detail(
            "workload.conf is loaded",
            [f"keys={len(values)}", f"path={case_dir / 'workload.conf'}"],
        ),
        "the regression runner owns the phase checks and uses the existing E2E helper functions",
    )
    run_step(
        result,
        "root privileges",
        module.require_root,
        expected_found_detail("uid=0", ["uid=0"]),
        "opencode payload capture requires eBPF/seccomp privileges",
    )
    actraild, actrailctl, actrailviewer = require_actrail_binaries(result, module, env.bin_dir)
    actrailweb = require_actrailweb_binary(result, module, env.bin_dir)
    tls_probe_point_finder = module.require_binary(env.bin_dir, "tls-probe-point-finder")
    tls_runtime = run_step(
        result,
        "opencode TLS runtime",
        lambda: resolve_opencode_tls_runtime(module, workload, tls_probe_point_finder),
        lambda tls: expected_found_detail(
            "TLS runtime discovery chooses the capture source",
            [opencode_tls_detail(tls)],
        ),
        "TLS runtime discovery decides whether request/response payloads come from BoringSSL or socket fallback",
    )
    resolved_config = None
    run_step(
        result,
        "default operator config",
        lambda: module.read_config(env.default_operator_config_path()),
        lambda values: expected_found_detail(
            "default operator config can be parsed",
            [f"keys={len(values)}", f"path={env.default_operator_config_path()}"],
        ),
        "the case uses the static default config and launch-time TLS auto discovery",
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
        finish_opencode_capture(
            env,
            result,
            module,
            workload,
            resolved_config,
            actrailctl,
            actrailviewer,
            actrailweb,
            tls_runtime,
        )
    finally:
        if daemon is not None:
            module.stop_process(
                daemon,
                float(module.required(workload, "daemon_stop_timeout_seconds")),
            )


def finish_opencode_capture(
    env,
    result: CaseResult,
    module,
    workload: dict[str, str],
    resolved_config: Path | None,
    actrailctl: Path,
    actrailviewer: Path,
    actrailweb: Path,
    tls_runtime,
) -> None:
    trace_id, launch_output = run_opencode_launch_step(
        result,
        module,
        actrailctl,
        resolved_config,
        workload,
    )
    payloads = run_step(
        result,
        "payload rows",
        lambda: module.wait_for_payloads_any(
            actrailctl,
            actrailviewer,
            resolved_config,
            trace_id,
            int(module.required(workload, "drain_attempts")),
            float(module.required(workload, "drain_sleep_seconds")),
            module.required(workload, "payload_head"),
            module.accepted_payload_fragments(tls_runtime),
        ),
        lambda rows: expected_found_detail(
            "viewer returns accepted payload rows",
            [f"trace_id=trace-{trace_id}", f"payload_output_bytes={len(rows.encode('utf-8'))}"],
        ),
        "payload table contains a complete outbound request row for an accepted source",
        progress=True,
    )
    payload_count = run_step(
        result,
        "provider request evidence",
        lambda: module.require_complete_payload_rows_any(
            payloads,
            module.accepted_payload_sources(tls_runtime),
            direction="outbound",
        ),
        lambda count: expected_found_detail("complete outbound payload segments exist", [f"opencode_payload_segments={count}"]),
        "viewer observed complete outbound payload rows for the pinned provider request",
    )
    response_count = run_step(
        result,
        "TLS response payload",
        lambda: module.require_tls_response_payloads(payloads, tls_runtime),
        lambda count: expected_found_detail("inbound TLS response payload segments are accounted for", [f"opencode_response_payload_segments={count}"]),
        "BoringSSL TLS runtime exposes inbound response plaintext when TLS capture is enabled",
    )
    actions = run_step(
        result,
        "semantic actions",
        lambda: module.wait_for_llm_exchange_actions(
            actrailviewer,
            resolved_config,
            trace_id,
            int(module.required(workload, "drain_attempts")),
            float(module.required(workload, "drain_sleep_seconds")),
        ),
        lambda rows: expected_found_detail(
            "viewer returns semantic llm.request and llm.response actions",
            [f"trace_id=trace-{trace_id}", f"action_output_bytes={len(rows.encode('utf-8'))}"],
        ),
        "semantic action projection ran after payload ingestion",
        progress=True,
    )
    run_step(
        result,
        "complete LLM exchange actions",
        lambda: module.require_complete_llm_exchange(actions),
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
    otel = run_step(
        result,
        "OTEL export",
        lambda: module.export_otel(
            actrailviewer,
            resolved_config,
            trace_id,
            Path(module.required(workload, "otel_output_path")),
        ),
        expected_found_detail("OTEL JSON export is written", [f"path={module.required(workload, 'otel_output_path')}"]),
        "actrailviewer exported the trace to OTEL JSON",
    )
    request_span_count = run_step(
        result,
        "llm.request OTEL span",
        lambda: module.require_otel_span(otel, "llm.request"),
        lambda count: expected_found_detail("OTEL contains llm.request spans", [f"opencode_llm_request_spans={count}"]),
        "OTEL export contains at least one semantic llm.request span",
    )
    response_span_count = run_step(
        result,
        "llm.response OTEL span",
        lambda: module.require_otel_span(otel, "llm.response"),
        lambda count: expected_found_detail("OTEL contains llm.response spans", [f"opencode_llm_response_spans={count}"]),
        "OTEL export contains at least one semantic llm.response span",
    )
    _, evidence_text = capture_stdout(
        lambda: module.emit_llm_otel_evidence(
            otel,
            int(module.required(workload, "evidence_text_max_chars")),
        )
    )
    result.stdout_tail = env.output_tail(
        opencode_output_summary(
            trace_id,
            payload_count,
            response_count,
            request_span_count,
            response_span_count,
            launch_output,
            evidence_text,
        )
    )
    result.add_check(
        "LLM exchange content",
        PASS
        if (
            "evidence.llm_request.body_attributes=omitted" in evidence_text
            and "evidence.llm_response.body_attributes=omitted" in evidence_text
        )
        else FAIL,
        expected_found_detail(
            "OTEL evidence includes request and response payload metadata",
            evidence_summary_facts(
                evidence_text,
                (
                    "evidence.llm_request.model=",
                    "evidence.llm_request.route=",
                    "evidence.llm_request.source=",
                    "evidence.llm_request.payload_bytes=",
                    "evidence.llm_request.body_attributes=",
                    "evidence.llm_response.model=",
                    "evidence.llm_response.source=",
                    "evidence.llm_response.payload_bytes=",
                    "evidence.llm_response.body_attributes=",
                ),
            ),
        ),
        "OTEL evidence must include parsed llm.request metadata without duplicated body content",
    )


def resolve_opencode_tls_runtime(module, workload: dict[str, str], tls_probe_point_finder: Path):
    entry = module.require_opencode_entry()
    return module.resolve_optional_opencode_tls_runtime(entry, workload, tls_probe_point_finder)
