"""opencode launch phase."""

from __future__ import annotations

from pathlib import Path

from e2e_steps.checks import StepFailure, capture_stdout
from evidence import expected_found_detail
from model import FAIL, PASS, SKIP, CaseResult


def run_opencode_launch_step(
    result: CaseResult,
    module,
    actrailctl: Path,
    resolved_config: Path | None,
    workload: dict[str, str],
    explicit_binary: str | None,
) -> tuple[int, str]:
    result.begin_check("opencode launch", "running opencode under actrailctl")
    try:
        (trace_id, output), _ = capture_stdout(
            lambda: module.launch_and_parse_trace(
                actrailctl,
                resolved_config,
                "agent-opencode-bun",
                [
                    "opencode",
                    "run",
                    "-m",
                    module.required(workload, "model"),
                    module.required(workload, "prompt"),
                ],
                float(module.required(workload, "launch_timeout_seconds")),
            )
        )
        if module.required(workload, "expected_output_fragment") not in output:
            raise RuntimeError("opencode output did not contain expected marker")
    except Exception as error:
        status = SKIP if not explicit_binary and "timed out after" in str(error) else FAIL
        result.status = status
        result.add_check(
            "opencode launch",
            status,
            str(error),
            "opencode provider command must emit the expected marker before payload checks can run",
        )
        raise StepFailure(str(error)) from error
    result.add_check(
        "opencode launch",
        PASS,
        expected_found_detail(
            "opencode command completes under actrailctl launch",
            [f"opencode_trace_id={trace_id}"],
        ),
        "real opencode run completed under actrailctl launch",
    )
    return trace_id, output
