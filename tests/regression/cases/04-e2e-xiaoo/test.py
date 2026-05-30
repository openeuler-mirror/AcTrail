"""xiaoO LLM payload regression case."""

from __future__ import annotations

import os
from pathlib import Path

from e2e_steps.checks import StepFailure
from e2e_steps.loader import load_package
from evidence import expected_found_detail
from model import FAIL, PASS, SKIP, WARN, CaseResult
from workload_config import read_config, required

CASE_DIR = Path(__file__).resolve().parent
DIRECT_STEPS = load_package("regression_e2e_xiaoo_direct_steps", CASE_DIR / "direct_steps")
run_direct_xiaoo_case = DIRECT_STEPS.run_direct_xiaoo_case


CASE_ID = "e2e-xiaoo"
TITLE = "E2E with xiaoO LLM request capture"
SUITES = {"quick", "agent", "payload", "full"}
WORKLOAD_CONFIG = "tests/agent-trace/xiaoo-rustls/workload.conf"


def run(env) -> CaseResult:
    result = CaseResult(CASE_ID, TITLE, PASS, 0.0)
    configured = os.environ.get("XIAOO_BINARY")
    candidates = xiaoo_candidates(env, configured)
    if not candidates:
        result.status = FAIL if configured else SKIP
        detail = f"XIAOO_BINARY is not executable: {configured}" if configured else "xiaoo is not on PATH"
        result.add_check(
            "xiaoO existence",
            result.status,
            expected_found_detail(
                "xiaoO executable is available",
                [
                    f"configured={configured or 'unset'}",
                    f"found={detail}",
                ],
            ),
            "real xiaoO provider traffic cannot be generated without the CLI/binary",
        )
        return result
    result.add_check(
        "xiaoO existence",
        PASS,
        expected_found_detail(
            "xiaoO executable is available",
            [f"candidate={path}" for path in candidates],
        ),
        "the case can launch a real xiaoO process",
    )
    tui = env.which("xiaoo-tui")
    result.add_check(
        "xiaoO TUI existence",
        PASS if tui else WARN,
        expected_found_detail(
            "optional xiaoO TUI discovery is recorded",
            [f"path={tui or 'missing optional xiaoo-tui'}"],
        ),
        "records whether the interactive frontend is installed; this payload case uses the CLI",
    )
    workload = read_config(env.repo_root / WORKLOAD_CONFIG)
    availability = run_xiaoo_availability_check(result, env, candidates[0], workload)
    if availability == SKIP:
        return result
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
        run_direct_xiaoo_case(env, result, configured, candidates[0])
    except StepFailure:
        return result
    if any(check.status == FAIL for check in result.checks):
        result.status = FAIL
    return result


def run_xiaoo_availability_check(
    result: CaseResult,
    env,
    xiaoo_binary: Path,
    workload: dict[str, str],
) -> str:
    result.begin_check("xiaoO availability", "running direct xiaoO marker check")
    availability = env.run(
        [
            str(xiaoo_binary),
            "run",
            "--no-tools",
            "--max-turns",
            required(workload, "availability_max_turns"),
            "--prompt",
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
            "xiaoO availability",
            SKIP,
            expected_found_detail(
                "direct xiaoO output contains configured availability marker",
                [
                    f"exit={availability.returncode}",
                    f"marker_present={required(workload, 'availability_marker') in availability.output}",
                ],
            ),
            "direct xiaoO run did not return the configured availability marker; "
            "fix xiaoO provider credentials, default model access, or network/proxy access "
            "before running AcTrail capture",
        )
        return SKIP
    result.add_check(
        "xiaoO availability",
        PASS,
        expected_found_detail(
            "direct xiaoO output contains configured availability marker",
            [
                f"exit={availability.returncode}",
                f"marker={required(workload, 'availability_marker')}",
            ],
        ),
        "xiaoO default provider configuration is usable before AcTrail launch/capture starts",
    )
    return PASS


def xiaoo_candidates(env, configured: str | None) -> list[Path]:
    if configured:
        resolved = env.resolve_executable_reference(configured)
        return [resolved] if resolved else []
    return env.executable_candidates("xiaoo")
