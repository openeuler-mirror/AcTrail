"""Direct xiaoO E2E phase runner."""

from __future__ import annotations

import os
from pathlib import Path

from e2e_steps.binaries import require_actrail_binaries
from e2e_steps.checks import capture_stdout, run_step
from e2e_steps.loader import load_module
from evidence import evidence_summary_facts, expected_found_detail
from model import FAIL, PASS, CaseResult

from .phases import run_xiaoo_actions_step, run_xiaoo_launch_step
from .reporting import xiaoo_output_summary, xiaoo_tls_detail


def run_direct_xiaoo_case(
    env,
    result: CaseResult,
    configured: str | None,
    selected: Path,
) -> None:
    module = load_module(
        "regression_xiaoo_rustls_run_e2e",
        env.repo_root / "tests/agent-trace/xiaoo-rustls/run_e2e.py",
    )
    old_binary = os.environ.get("XIAOO_BINARY")
    os.environ["XIAOO_BINARY"] = str(selected)
    try:
        run_loaded_xiaoo_case(env, result, module, configured)
    finally:
        restore_optional_env("XIAOO_BINARY", old_binary)


def run_loaded_xiaoo_case(env, result: CaseResult, module, configured: str | None) -> None:
    case_dir = env.repo_root / "tests/agent-trace/xiaoo-rustls"
    daemon = None
    result.command = ["direct", "xiaoo-rustls"]
    workload = run_step(
        result,
        "xiaoO workload config",
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
        "xiaoO payload capture requires eBPF/seccomp privileges",
    )
    actraild, actrailctl, actrailviewer = require_actrail_binaries(result, module, env.bin_dir)
    xiaoo_binary = run_step(
        result,
        "xiaoO selected binary",
        lambda: module.resolve_xiaoo_binary(module.required(workload, "xiaoo_binary")),
        lambda path: expected_found_detail("selected xiaoO executable resolves", [f"path={path}"]),
        "the selected xiaoO executable is resolved before TLS/runtime checks",
    )
    tls_runtime = run_step(
        result,
        "xiaoO TLS runtime",
        lambda: module.resolve_optional_xiaoo_tls_runtime(xiaoo_binary, workload),
        lambda tls: expected_found_detail(
            "TLS runtime discovery chooses the capture source",
            [xiaoo_tls_detail(tls)],
        ),
        "rustls symbol discovery decides whether HTTPS payloads can be decoded as plaintext",
    )
    resolved_config = Path(module.required(workload, "resolved_config_path"))
    run_step(
        result,
        "operator config",
        lambda: render_xiaoo_config(module, case_dir, resolved_config, xiaoo_binary, tls_runtime),
        lambda path: expected_found_detail("resolved operator config is rendered", [f"path={path}"]),
        "the case renders a concrete operator config for the selected xiaoO runtime",
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
        finish_xiaoo_capture(
            env,
            result,
            module,
            workload,
            resolved_config,
            actrailctl,
            actrailviewer,
            xiaoo_binary,
            tls_runtime,
            configured,
        )
    finally:
        if daemon is not None:
            module.stop_process(
                daemon,
                float(module.required(workload, "daemon_stop_timeout_seconds")),
            )


def finish_xiaoo_capture(
    env,
    result: CaseResult,
    module,
    workload: dict[str, str],
    resolved_config: Path,
    actrailctl: Path,
    actrailviewer: Path,
    xiaoo_binary: Path,
    tls_runtime,
    configured: str | None,
) -> None:
    trace_id, launch_output = run_xiaoo_launch_step(
        result,
        module,
        actrailctl,
        resolved_config,
        workload,
        xiaoo_binary,
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
        "payload capture",
        lambda: module.require_complete_payload_rows_any(
            payloads,
            module.accepted_payload_sources(tls_runtime),
            direction="outbound",
        ),
        lambda count: expected_found_detail("complete outbound payload segments exist", [f"xiaoo_payload_segments={count}"]),
        "viewer observed complete outbound payload rows for xiaoO provider traffic",
    )
    actions = run_xiaoo_actions_step(
        result,
        module,
        actrailviewer,
        resolved_config,
        trace_id,
        workload,
        tls_runtime,
        configured,
    )
    run_step(
        result,
        "complete LLM exchange actions",
        lambda: module.require_complete_llm_exchange(actions),
        expected_found_detail(
            "complete successful llm.request and llm.response exist",
            ["complete successful llm.request", "complete successful llm.response"],
        ),
        "the action table contains a complete successful semantic request/response exchange",
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
        lambda count: expected_found_detail("OTEL contains llm.request spans", [f"xiaoo_llm_request_spans={count}"]),
        "OTEL export contains semantic llm.request spans for xiaoO provider traffic",
    )
    response_span_count = run_step(
        result,
        "llm.response OTEL span",
        lambda: module.require_otel_span(otel, "llm.response"),
        lambda count: expected_found_detail("OTEL contains llm.response spans", [f"xiaoo_llm_response_spans={count}"]),
        "OTEL export contains semantic llm.response spans for xiaoO provider traffic",
    )
    _, evidence_text = capture_stdout(
        lambda: module.emit_llm_otel_evidence(
            otel,
            int(module.required(workload, "evidence_text_max_chars")),
        )
    )
    result.stdout_tail = env.output_tail(
        xiaoo_output_summary(
            trace_id,
            payload_count,
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


def render_xiaoo_config(module, case_dir: Path, resolved_config: Path, xiaoo_binary: Path, tls_runtime):
    module.render_config(
        case_dir / "operator.conf",
        resolved_config,
        module.xiaoo_config_replacements(xiaoo_binary, tls_runtime),
    )
    return resolved_config


def restore_optional_env(name: str, value: str | None) -> None:
    if value is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = value
