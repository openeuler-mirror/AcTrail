"""Direct HTTP projection E2E phase runner."""

from __future__ import annotations

from pathlib import Path

from e2e_steps.binaries import require_actrail_binaries, require_actrailweb_binary
from e2e_steps.checks import StepFailure, capture_stdout, run_step
from e2e_steps.loader import load_module
from evidence import evidence_summary_facts, expected_found_detail
from model import FAIL, PASS, CaseResult


def run_direct_http_projection_case(env, result: CaseResult) -> None:
    module = load_module(
        "regression_http_llm_projection_run_e2e",
        env.regression_root / "cases/08-http-llm-projection/run_e2e.py",
    )
    case_dir = env.regression_root / "cases/08-http-llm-projection"
    config = env.repo_root / "tests/payload/http-local/operator.conf"
    settings = run_step(
        result,
        "HTTP projection workload config",
        lambda: module.read_config(case_dir / "workload.conf"),
        lambda values: expected_found_detail(
            "workload.conf is loaded",
            [f"keys={len(values)}", f"path={case_dir / 'workload.conf'}"],
        ),
        "the regression runner owns the phase checks and uses the existing E2E helper functions",
    )
    daemon = None
    result.command = ["direct", "http-llm-projection"]
    actraild, actrailctl, actrailviewer = require_actrail_binaries(result, module, env.bin_dir)
    actrailweb = require_actrailweb_binary(result, module, env.bin_dir)
    workload = case_dir / "workload.py"
    run_step(
        result,
        "HTTP projection operator config",
        lambda: module.read_config(config),
        lambda values: expected_found_detail(
            "HTTP projection operator config can be parsed",
            [f"keys={len(values)}", f"path={config}"],
        ),
        "the case uses a checked-in migrated config for local HTTP payload projection",
    )
    run_step(
        result,
        "clean previous state",
        lambda: module.clean_configured_paths(actrailctl, config),
        expected_found_detail("previous configured state is removed", ["clean complete"]),
        "previous sockets, pid files, storage, and exports were removed",
    )
    daemon = run_step(
        result,
        "actraild daemon",
        lambda: module.start_daemon(
            actraild,
            config,
            float(module.required(settings, "daemon_ready_timeout_seconds")),
        ),
        expected_found_detail("actraild reports daemon listening", ["daemon listening"]),
        "the capture daemon accepted the local HTTP payload config",
        progress=True,
    )
    try:
        finish_http_projection_capture(
            env,
            result,
            module,
            settings,
            workload,
            config,
            actrailctl,
            actrailviewer,
            actrailweb,
        )
    finally:
        if daemon is not None:
            module.stop_process(
                daemon,
                float(module.required(settings, "daemon_stop_timeout_seconds")),
            )


def finish_http_projection_capture(
    env,
    result: CaseResult,
    module,
    settings: dict[str, str],
    workload: Path,
    config: Path | None,
    actrailctl: Path,
    actrailviewer: Path,
    actrailweb: Path,
) -> None:
    trace_id, workload_pid = run_http_projection_launch_step(
        result,
        module,
        actrailctl,
        config,
        settings,
        workload,
    )
    run_step(
        result,
        "launch root identity",
        lambda: require_launch_root_identity(
            module,
            actrailviewer,
            config,
            trace_id,
            workload_pid,
        ),
        lambda root: expected_found_detail(
            "trace root identity owns the launched workload namespace PID",
            [
                f"trace_id=trace-{trace_id}",
                f"workload_pid={workload_pid}",
                *root.facts(workload_pid),
            ],
        ),
        "actrailctl launch should track the launched process as the trace root",
    )
    payloads = run_step(
        result,
        "payload rows",
        lambda: module.wait_for_payloads(
            actrailctl,
            actrailviewer,
            config,
            trace_id,
            int(module.required(settings, "drain_attempts")),
            float(module.required(settings, "drain_sleep_seconds")),
            module.required(settings, "payload_head"),
            ["Syscall", "socket-syscall", "Complete", "success"],
        ),
        lambda rows: expected_found_detail(
            "viewer returns socket payload rows",
            [f"trace_id=trace-{trace_id}", f"payload_output_bytes={len(rows.encode('utf-8'))}"],
        ),
        "payload table contains complete outbound socket-syscall rows",
        progress=True,
    )
    payload_count = run_step(
        result,
        "socket payload source",
        lambda: module.require_complete_outbound_socket_payload_rows(payloads),
        lambda count: expected_found_detail("complete outbound socket payload segments exist", [f"http_llm_projection_payload_segments={count}"]),
        "viewer observed complete outbound Syscall/socket-syscall payload rows",
    )
    actions = run_step(
        result,
        "semantic actions",
        lambda: module.wait_for_actions(
            actrailviewer,
            config,
            trace_id,
            int(module.required(settings, "drain_attempts")),
            float(module.required(settings, "drain_sleep_seconds")),
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
    run_step(
        result,
        "Web action-tree reachability",
        lambda: module.require_web_action_tree_projection(
            actrailweb,
            config,
            trace_id,
            float(module.required(settings, "daemon_ready_timeout_seconds")),
            float(module.required(settings, "drain_sleep_seconds")),
            required_reachable_kinds=("llm.request", "http.message"),
            forbidden_root_linkless_kinds=("http.message",),
            required_parent_child_kinds=(("command.invocation", "http.message"),),
        ),
        lambda summary: expected_found_detail(
            "web action-tree reaches every display action and keeps HTTP under its command display parent",
            [
                f"actions={summary['action_count']}",
                f"reachable={summary['reachable_count']}",
                f"http_messages={summary['kind_counts'].get('http.message', 0)}",
                f"root_linkless={summary['root_linkless_count']}",
                f"command_http_children={summary['parent_child_kind_counts'].get(('command.invocation', 'http.message'), 0)}",
            ],
        ),
        "actrailweb action-tree API exposes HTTP semantic actions without leaving them as root fallback nodes",
    )
    otel = run_step(
        result,
        "OTEL export",
        lambda: module.export_otel(
            actrailviewer,
            config,
            trace_id,
            Path(module.required(settings, "otel_output_path")),
        ),
        expected_found_detail("OTEL JSON export is written", [f"path={module.required(settings, 'otel_output_path')}"]),
        "actrailviewer exported the trace to OTEL JSON",
    )
    span_count = run_step(
        result,
        "llm.request OTEL span",
        lambda: module.require_otel_span(otel, "llm.request"),
        lambda count: expected_found_detail("OTEL contains llm.request spans", [f"http_llm_projection_spans={count}"]),
        "pretty OTEL export contains a semantic llm.request span",
    )
    marker = module.required(settings, "marker")
    run_step(
        result,
        "OTEL payload marker",
        lambda: module.require_http_llm_span(otel, marker, module.required(settings, "model")),
        expected_found_detail("OTEL llm.request span contains payload metadata", [f"http_llm_projection_marker={marker}"]),
        "the exported llm.request semantic span contains payload metadata without duplicating prompt text",
    )
    _, evidence_text = capture_stdout(
        lambda: module.emit_llm_otel_evidence(
            otel,
            int(module.required(settings, "evidence_text_max_chars")),
        )
    )
    result.stdout_tail = env.output_tail(
        http_projection_output_summary(trace_id, payload_count, span_count, marker, evidence_text)
    )
    result.add_check(
        "LLM request content",
        PASS if "evidence.llm_request.body_attributes=omitted" in evidence_text else FAIL,
        expected_found_detail(
            "OTEL evidence includes model, route, source, payload bytes, and response status",
            evidence_summary_facts(
                evidence_text,
                (
                    "evidence.llm_request.model=",
                    "evidence.llm_request.route=",
                    "evidence.llm_request.source=",
                    "evidence.llm_request.payload_bytes=",
                    "evidence.llm_request.body_attributes=",
                    "evidence.llm_response=",
                ),
            ),
        ),
        "OTEL evidence must include parsed llm.request metadata without duplicated body content",
    )


def run_http_projection_launch_step(
    result: CaseResult,
    module,
    actrailctl: Path,
    config: Path | None,
    settings: dict[str, str],
    workload: Path,
) -> tuple[int, int]:
    result.begin_check("HTTP projection launch", "running local workload under actrailctl")
    try:
        (trace_id, output), _ = capture_stdout(
            lambda: module.launch_and_parse_trace(
                actrailctl,
                config,
                "http-llm-projection",
                module.workload_argv(workload, settings),
                float(module.required(settings, "launch_timeout_seconds")),
            )
        )
        if "llm projection workload complete" not in output:
            raise RuntimeError("workload did not report completion")
        workload_pid = module.parse_workload_pid(output)
    except Exception as error:
        result.status = FAIL
        result.add_check(
            "HTTP projection launch",
            FAIL,
            str(error),
            "local OpenAI-style HTTP request must complete before payload checks can run",
        )
        raise StepFailure(str(error)) from error
    result.add_check(
        "HTTP projection launch",
        PASS,
        expected_found_detail(
            "local OpenAI-style HTTP request completes under actrailctl launch",
            [
                f"http_llm_projection_trace_id={trace_id}",
                f"http_llm_projection_workload_pid={workload_pid}",
            ],
        ),
        "local OpenAI-style HTTP request completed under AcTrail",
    )
    return trace_id, workload_pid


def require_launch_root_identity(
    module,
    actrailviewer: Path,
    config: Path | None,
    trace_id: int,
    workload_pid: int,
):
    summary = module.trace_summary(actrailviewer, config, trace_id)
    return module.require_launch_root_identity(config, trace_id, summary, workload_pid)


def http_projection_output_summary(
    trace_id: int,
    payload_count: int,
    span_count: int,
    marker: str,
    evidence_text: str,
) -> str:
    return (
        f"http_llm_projection_trace_id={trace_id}\n"
        f"http_llm_projection_payload_segments={payload_count}\n"
        f"http_llm_projection_spans={span_count}\n"
        f"http_llm_projection_marker={marker}\n"
        f"{evidence_text}"
    )
