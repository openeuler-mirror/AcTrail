"""Direct opencode E2E phase runner."""

from __future__ import annotations

import os
from pathlib import Path

from e2e_steps.binaries import require_actrail_binaries
from e2e_steps.checks import capture_stdout, run_step
from e2e_steps.loader import load_module
from evidence import evidence_summary_facts, expected_found_detail
from model import FAIL, PASS, CaseResult

from .launch import run_opencode_launch_step
from .reporting import opencode_output_summary, opencode_tls_detail


def run_direct_opencode_case(
    env,
    result: CaseResult,
    explicit_binary: str | None,
    entry: Path,
) -> None:
    module = load_module(
        "regression_opencode_bun_run_e2e",
        env.repo_root / "tests/agent-trace/opencode-bun/run_e2e.py",
    )
    old_path = os.environ.get("PATH", "")
    old_override = os.environ.get("OPENCODE_BIN_PATH")
    os.environ["PATH"] = f"{entry.parent}{os.pathsep}{old_path}"
    if explicit_binary:
        os.environ["OPENCODE_BIN_PATH"] = explicit_binary
    else:
        os.environ.pop("OPENCODE_BIN_PATH", None)
    try:
        run_loaded_opencode_case(env, result, module, explicit_binary)
    finally:
        os.environ["PATH"] = old_path
        restore_optional_env("OPENCODE_BIN_PATH", old_override)


def run_loaded_opencode_case(env, result: CaseResult, module, explicit_binary: str | None) -> None:
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
    tls_runtime = run_step(
        result,
        "opencode TLS runtime",
        lambda: resolve_opencode_tls_runtime(module, workload, case_dir, env.repo_root),
        lambda tls: expected_found_detail(
            "TLS runtime discovery chooses the capture source",
            [opencode_tls_detail(tls)],
        ),
        "TLS runtime discovery decides whether request/response payloads come from BoringSSL or socket fallback",
    )
    resolved_config = Path(module.required(workload, "resolved_config_path"))
    run_step(
        result,
        "operator config",
        lambda: render_opencode_config(module, case_dir, resolved_config, tls_runtime),
        lambda path: expected_found_detail("resolved operator config is rendered", [f"path={path}"]),
        "the case renders a concrete operator config for the selected opencode runtime",
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
            tls_runtime,
            explicit_binary,
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
    resolved_config: Path,
    actrailctl: Path,
    actrailviewer: Path,
    tls_runtime,
    explicit_binary: str | None,
) -> None:
    trace_id, launch_output = run_opencode_launch_step(
        result,
        module,
        actrailctl,
        resolved_config,
        workload,
        explicit_binary,
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
        lambda: module.wait_for_actions(
            actrailviewer,
            resolved_config,
            trace_id,
            int(module.required(workload, "drain_attempts")),
            float(module.required(workload, "drain_sleep_seconds")),
        ),
        lambda rows: expected_found_detail(
            "viewer returns semantic llm.request actions",
            [f"trace_id=trace-{trace_id}", f"action_output_bytes={len(rows.encode('utf-8'))}"],
        ),
        "semantic action projection ran after payload ingestion",
        progress=True,
    )
    run_step(
        result,
        "complete llm.request action",
        lambda: module.require_complete_llm_action(actions),
        expected_found_detail("complete successful llm.request exists", ["complete successful llm.request"]),
        "the action table contains a complete successful semantic request",
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
    span_count = run_step(
        result,
        "llm.request OTEL span",
        lambda: module.require_otel_span(otel, "llm.request"),
        lambda count: expected_found_detail("OTEL contains llm.request spans", [f"opencode_llm_request_spans={count}"]),
        "OTEL export contains at least one semantic llm.request span",
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
            span_count,
            launch_output,
            evidence_text,
        )
    )
    result.add_check(
        "LLM request content",
        PASS if "evidence.llm_request.body_text_json=" in evidence_text else FAIL,
        expected_found_detail(
            "OTEL evidence includes model, route, source, request body, and response status",
            evidence_summary_facts(
                evidence_text,
                (
                    "evidence.llm_request.model=",
                    "evidence.llm_request.route=",
                    "evidence.llm_request.source=",
                    "evidence.llm_request.body_text_json=",
                    "evidence.llm_response=",
                ),
            ),
        ),
        "OTEL evidence must include parsed llm.request content",
    )


def resolve_opencode_tls_runtime(module, workload: dict[str, str], case_dir: Path, repo: Path):
    entry = module.require_opencode_entry()
    configured_symbol_map = module.resolve_path(module.required(workload, "symbol_map_path"), repo)
    return module.resolve_optional_opencode_tls_runtime(entry, configured_symbol_map, workload)


def render_opencode_config(module, case_dir: Path, resolved_config: Path, tls_runtime):
    module.render_config(
        case_dir / "operator.conf",
        resolved_config,
        module.opencode_config_replacements(tls_runtime),
    )
    return resolved_config


def restore_optional_env(name: str, value: str | None) -> None:
    if value is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = value
