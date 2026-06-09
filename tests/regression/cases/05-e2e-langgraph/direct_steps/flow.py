"""Direct LangGraph E2E phase runner."""

from __future__ import annotations

from pathlib import Path

from e2e_steps.binaries import require_actrail_binaries
from e2e_steps.checks import StepFailure, capture_stdout, run_step
from e2e_steps.loader import load_module
from evidence import evidence_summary_facts, expected_found_detail
from model import FAIL, PASS, CaseResult


def run_direct_langgraph_case(
    env,
    result: CaseResult,
    workload: dict[str, str],
    python: str,
) -> None:
    module = load_module(
        "regression_langgraph_openai_run_e2e",
        env.repo_root / "tests/agent-trace/langgraph-openai/run_e2e.py",
    )
    run_loaded_langgraph_case(env, result, module, workload, python)


def run_loaded_langgraph_case(
    env,
    result: CaseResult,
    module,
    workload: dict[str, str],
    python: str,
) -> None:
    case_dir = env.repo_root / "tests/agent-trace/langgraph-openai"
    config = case_dir / "operator.conf"
    daemon = None
    result.command = ["direct", "langgraph-openai"]
    run_step(
        result,
        "root privileges",
        module.require_root,
        expected_found_detail("uid=0", ["uid=0"]),
        "LangGraph payload capture requires eBPF/seccomp privileges",
    )
    actraild, actrailctl, actrailviewer = require_actrail_binaries(result, module, env.bin_dir)
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
            float(module.required(workload, "daemon_ready_timeout_seconds")),
        ),
        expected_found_detail("actraild reports daemon listening", ["daemon listening"]),
        "the capture daemon accepted the LangGraph config",
        progress=True,
    )
    try:
        finish_langgraph_capture(
            env,
            result,
            module,
            workload,
            python,
            case_dir,
            config,
            actrailctl,
            actrailviewer,
        )
    finally:
        if daemon is not None:
            module.stop_process(
                daemon,
                float(module.required(workload, "daemon_stop_timeout_seconds")),
            )


def finish_langgraph_capture(
    env,
    result: CaseResult,
    module,
    workload: dict[str, str],
    python: str,
    case_dir: Path,
    config: Path,
    actrailctl: Path,
    actrailviewer: Path,
) -> None:
    trace_id, launch_output = run_langgraph_launch_step(
        result,
        module,
        actrailctl,
        config,
        workload,
        python,
        case_dir,
    )
    payloads = run_step(
        result,
        "payload rows",
        lambda: module.wait_for_payloads_any(
            actrailctl,
            actrailviewer,
            config,
            trace_id,
            int(module.required(workload, "drain_attempts")),
            float(module.required(workload, "drain_sleep_seconds")),
            module.required(workload, "payload_head"),
            module.accepted_payload_fragments(),
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
            module.accepted_payload_sources(),
            direction="outbound",
        ),
        lambda count: expected_found_detail("complete outbound payload segments exist", [f"langgraph_payload_segments={count}"]),
        "viewer observed complete outbound payload rows for the LangGraph provider call",
    )
    actions = run_step(
        result,
        "semantic actions",
        lambda: module.wait_for_llm_exchange_actions(
            actrailviewer,
            config,
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
            config,
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
        lambda count: expected_found_detail("OTEL contains llm.request spans", [f"langgraph_llm_request_spans={count}"]),
        "OTEL export contains semantic llm.request spans for the LangGraph provider call",
    )
    response_span_count = run_step(
        result,
        "llm.response OTEL span",
        lambda: module.require_otel_span(otel, "llm.response"),
        lambda count: expected_found_detail("OTEL contains llm.response spans", [f"langgraph_llm_response_spans={count}"]),
        "OTEL export contains semantic llm.response spans for the LangGraph provider call",
    )
    _, evidence_text = capture_stdout(
        lambda: module.emit_llm_otel_evidence(
            otel,
            int(module.required(workload, "evidence_text_max_chars")),
        )
    )
    result.stdout_tail = env.output_tail(
        langgraph_output_summary(
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


def run_langgraph_launch_step(
    result: CaseResult,
    module,
    actrailctl: Path,
    config: Path,
    workload: dict[str, str],
    python: str,
    case_dir: Path,
) -> tuple[int, str]:
    result.begin_check("LangGraph launch", "running workload under actrailctl")
    try:
        api_key_env = module.required(workload, "api_key_env")
        (trace_id, output), _ = capture_stdout(
            lambda: module.launch_and_parse_trace(
                actrailctl,
                config,
                "agent-langgraph-openai",
                [
                    python,
                    str(case_dir / "workload.py"),
                    "--prompt",
                    module.required(workload, "prompt"),
                    "--model",
                    module.required(workload, "model"),
                    "--api-url",
                    module.required(workload, "api_url"),
                    "--api-key-env",
                    api_key_env,
                ],
                float(module.required(workload, "launch_timeout_seconds")),
            )
        )
        if module.required(workload, "expected_output_fragment") not in output:
            raise RuntimeError("LangGraph output did not contain expected marker")
    except Exception as error:
        result.status = FAIL
        result.add_check(
            "LangGraph launch",
            FAIL,
            str(error),
            "LangGraph workload must emit the expected marker before payload checks can run",
        )
        raise StepFailure(str(error)) from error
    result.add_check(
        "LangGraph launch",
        PASS,
        expected_found_detail(
            "LangGraph workload completes under actrailctl launch",
            [f"langgraph_trace_id={trace_id}"],
        ),
        "real LangGraph workload completed under actrailctl launch",
    )
    return trace_id, output


def langgraph_output_summary(
    trace_id: int,
    payload_count: int,
    request_span_count: int,
    response_span_count: int,
    launch_output: str,
    evidence_text: str,
) -> str:
    return (
        f"langgraph_trace_id={trace_id}\n"
        f"langgraph_payload_segments={payload_count}\n"
        f"langgraph_llm_request_spans={request_span_count}\n"
        f"langgraph_llm_response_spans={response_span_count}\n"
        f"{launch_output}\n"
        f"{evidence_text}"
    )
