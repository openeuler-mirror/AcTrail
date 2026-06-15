"""xiaoO checked capture phases."""

from __future__ import annotations

from pathlib import Path

from e2e_steps.checks import StepFailure, capture_stdout
from evidence import expected_found_detail
from model import FAIL, PASS, SKIP, CaseResult


def run_xiaoo_launch_step(
    result: CaseResult,
    module,
    actrailctl: Path,
    resolved_config: Path | None,
    workload: dict[str, str],
    xiaoo_binary: Path,
) -> tuple[int, str]:
    result.begin_check("xiaoO launch", "running xiaoO under actrailctl")
    try:
        (trace_id, output), _ = capture_stdout(
            lambda: module.launch_and_parse_trace(
                actrailctl,
                resolved_config,
                "agent-xiaoo-rustls",
                [
                    str(xiaoo_binary),
                    "run",
                    "--no-tools",
                    "--max-turns",
                    "1",
                    "--prompt",
                    module.required(workload, "prompt"),
                ],
                float(module.required(workload, "launch_timeout_seconds")),
            )
        )
        if module.required(workload, "expected_output_fragment") not in output:
            raise RuntimeError("xiaoO output did not contain expected marker")
    except Exception as error:
        result.status = FAIL
        result.add_check(
            "xiaoO launch",
            FAIL,
            str(error),
            "xiaoO must emit the expected marker before payload checks can run",
        )
        raise StepFailure(str(error)) from error
    result.add_check(
        "xiaoO launch",
        PASS,
        expected_found_detail(
            "xiaoO command completes under actrailctl launch",
            [f"xiaoo_trace_id={trace_id}"],
        ),
        "real xiaoO run completed under actrailctl launch",
    )
    result.add_check(
        "xiaoO functionality",
        PASS,
        expected_found_detail(
            "xiaoO output contains expected marker",
            [f"marker={module.required(workload, 'expected_output_fragment')}"],
        ),
        "the monitored xiaoO agent returned the expected marker",
    )
    return trace_id, output


def run_xiaoo_actions_step(
    result: CaseResult,
    module,
    actrailviewer: Path,
    resolved_config: Path | None,
    trace_id: int,
    workload: dict[str, str],
    tls_runtime,
    configured: str | None,
) -> str:
    result.begin_check("semantic actions", "waiting for llm.request and llm.response")
    try:
        actions, _ = capture_stdout(
            lambda: module.wait_for_llm_exchange_actions(
                actrailviewer,
                resolved_config,
                trace_id,
                int(module.required(workload, "drain_attempts")),
                float(module.required(workload, "drain_sleep_seconds")),
            )
        )
    except Exception as error:
        result.status = FAIL
        result.add_check(
            "semantic actions",
            FAIL,
            str(error),
            "xiaoO rustls capture resolved TLS hooks, so llm.request/llm.response projection is required",
        )
        raise StepFailure(str(error)) from error
    result.add_check(
        "semantic actions",
        PASS,
        expected_found_detail(
            "viewer returns semantic llm.request and llm.response actions",
            [f"trace_id=trace-{trace_id}", f"action_output_bytes={len(actions.encode('utf-8'))}"],
        ),
        "semantic action projection ran after payload ingestion",
    )
    return actions
