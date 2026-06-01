"""Claude launch phase."""

from __future__ import annotations

from pathlib import Path

from e2e_steps.checks import StepFailure, capture_stdout
from evidence import expected_found_detail
from model import FAIL, PASS, CaseResult


def run_claude_launch_step(
    result: CaseResult,
    module,
    daemon,
    actrailctl: Path,
    resolved_config: Path,
    workload: dict[str, str],
) -> int:
    result.begin_check("Claude launch", "running claude under actrailctl")
    try:
        (trace_id, _), _ = capture_stdout(
            lambda: module.launch_and_parse_trace_with_daemon(
                daemon,
                actrailctl,
                resolved_config,
                "claude-code-real-e2e",
                ["claude", "-p", module.required(workload, "prompt")],
                float(module.required(workload, "claude_timeout_seconds")),
                float(module.required(workload, "launch_poll_interval_seconds")),
                float(module.required(workload, "launch_stop_timeout_seconds")),
            )
        )
    except Exception as error:
        result.status = FAIL
        result.add_check(
            "Claude launch",
            FAIL,
            str(error),
            "Claude must run under actrailctl launch before payload checks can run",
        )
        raise StepFailure(str(error)) from error
    result.add_check(
        "Claude launch",
        PASS,
        expected_found_detail(
            "`claude -p` completes under actrailctl launch",
            [f"claude_code_trace_id={trace_id}"],
        ),
        "real `claude -p` completed under actrailctl launch",
    )
    return trace_id
