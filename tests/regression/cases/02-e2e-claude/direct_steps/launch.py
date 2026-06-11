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
    resolved_config: Path | None,
    workload: dict[str, str],
) -> int:
    return run_claude_launch_mode_step(
        result,
        module,
        daemon,
        actrailctl,
        resolved_config,
        "Claude launch",
        "`claude -p`",
        "claude-code-real-e2e",
        ["claude", "-p", module.required(workload, "prompt")],
        module.required(workload, "prompt_marker"),
        workload,
    )


def run_claude_interactive_launch_step(
    result: CaseResult,
    module,
    daemon,
    actrailctl: Path,
    resolved_config: Path | None,
    workload: dict[str, str],
) -> int:
    return run_claude_launch_mode_step(
        result,
        module,
        daemon,
        actrailctl,
        resolved_config,
        "Claude interactive launch",
        '`claude "<prompt>"`',
        "claude-code-interactive-entry-e2e",
        ["claude", module.required(workload, "interactive_prompt")],
        module.required(workload, "interactive_prompt_marker"),
        workload,
    )


def run_claude_launch_mode_step(
    result: CaseResult,
    module,
    daemon,
    actrailctl: Path,
    resolved_config: Path | None,
    check_name: str,
    label: str,
    trace_name: str,
    argv: list[str],
    expected_marker: str,
    workload: dict[str, str],
) -> int:
    result.begin_check(check_name, f"running {label} under actrailctl")
    try:
        (trace_id, output), _ = capture_stdout(
            lambda: module.launch_and_parse_trace_with_daemon(
                daemon,
                actrailctl,
                resolved_config,
                trace_name,
                argv,
                float(module.required(workload, "claude_timeout_seconds")),
                float(module.required(workload, "launch_poll_interval_seconds")),
                float(module.required(workload, "launch_stop_timeout_seconds")),
            )
        )
        if expected_marker not in output:
            raise RuntimeError(f"{label} output did not contain expected marker")
    except Exception as error:
        result.status = FAIL
        result.add_check(
            check_name,
            FAIL,
            str(error),
            "Claude must run under actrailctl launch before payload checks can run",
        )
        raise StepFailure(str(error)) from error
    result.add_check(
        check_name,
        PASS,
        expected_found_detail(
            f"{label} completes under actrailctl launch",
            [f"claude_code_trace_id={trace_id}"],
        ),
        f"real {label} completed under actrailctl launch",
    )
    return trace_id
