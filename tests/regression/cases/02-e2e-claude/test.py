"""Claude Code LLM payload regression case."""

from __future__ import annotations

from pathlib import Path

from e2e_steps.checks import StepFailure
from e2e_steps.loader import load_package
from evidence import expected_found_detail
from model import FAIL, PASS, SKIP, CaseResult
from workload_config import read_config, required

CASE_DIR = Path(__file__).resolve().parent
DIRECT_STEPS = load_package("regression_e2e_claude_direct_steps", CASE_DIR / "direct_steps")
run_direct_claude_case = DIRECT_STEPS.run_direct_claude_case


CASE_ID = "e2e-claude"
TITLE = "E2E with Claude Code LLM exchange capture"
SUITES = {"quick", "agent", "payload", "full"}
WORKLOAD_CONFIG = "tests/payload/claude-code/workload.conf"


def run(env) -> CaseResult:
    result = CaseResult(CASE_ID, TITLE, PASS, 0.0)
    claude = env.which("claude")
    if not claude:
        result.status = SKIP
        result.add_check(
            "claude existence",
            SKIP,
            expected_found_detail("claude executable on PATH", ["found=missing"]),
            "real Claude Code traffic cannot be generated without the CLI",
        )
        return result
    result.add_check(
        "claude existence",
        PASS,
        expected_found_detail("claude executable on PATH", [f"path={claude}"]),
        "the case can launch a real Claude Code process instead of a synthetic workload",
    )
    workload = read_config(env.repo_root / WORKLOAD_CONFIG)
    result.begin_check("claude availability", "running direct `claude -p` marker check")
    availability = env.run(
        [
            claude,
            "-p",
            required(workload, "availability_prompt"),
        ],
        timeout=float(required(workload, "availability_timeout_seconds")),
    )
    if (
        availability.returncode != 0
        or required(workload, "availability_marker") not in availability.output
    ):
        result.status = SKIP
        result.command = availability.command
        result.stdout_tail = env.output_tail(availability.stdout)
        result.stderr_tail = env.output_tail(availability.stderr)
        result.add_check(
            "claude availability",
            SKIP,
            expected_found_detail(
                "direct `claude -p` output contains configured availability marker",
                [
                    f"exit={availability.returncode}",
                    f"marker_present={required(workload, 'availability_marker') in availability.output}",
                ],
            ),
            "direct `claude -p` did not return the configured availability marker; "
            "fix Claude authentication/default model access before running AcTrail capture",
        )
        return result
    result.add_check(
        "claude availability",
        PASS,
        expected_found_detail(
            "direct `claude -p` output contains configured availability marker",
            [
                f"exit={availability.returncode}",
                f"marker={required(workload, 'availability_marker')}",
            ],
        ),
        "Claude CLI default configuration is usable before AcTrail launch/capture starts",
    )
    if not env.release_binaries_ready():
        result.status = SKIP
        result.add_check(
            "release binaries",
            SKIP,
            expected_found_detail("compiled release binaries are present", ["missing one or more release binaries"]),
            "the E2E must use compiled AcTrail binaries",
        )
        return result
    try:
        run_direct_claude_case(env, result, workload)
    except StepFailure:
        return result
    if any(check.status == FAIL for check in result.checks):
        result.status = FAIL
    return result
