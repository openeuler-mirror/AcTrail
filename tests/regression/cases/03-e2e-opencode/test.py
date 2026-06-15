"""opencode LLM payload regression case."""

from __future__ import annotations

from pathlib import Path

from e2e_steps.checks import StepFailure
from e2e_steps.loader import load_package
from evidence import expected_found_detail
from model import FAIL, PASS, SKIP, CaseResult

CASE_DIR = Path(__file__).resolve().parent
DIRECT_STEPS = load_package("regression_e2e_opencode_direct_steps", CASE_DIR / "direct_steps")
run_direct_opencode_case = DIRECT_STEPS.run_direct_opencode_case


CASE_ID = "e2e-opencode"
TITLE = "E2E with opencode LLM exchange capture"
SUITES = {"quick", "agent", "payload", "full"}


def run(env) -> CaseResult:
    result = CaseResult(CASE_ID, TITLE, PASS, 0.0)
    launchers = env.executable_candidates("opencode")
    if not launchers:
        result.status = SKIP
        result.add_check(
            "opencode existence",
            SKIP,
            expected_found_detail("opencode executable on PATH", ["found=missing"]),
            "real opencode provider traffic cannot be generated without the CLI",
        )
        return result
    result.add_check(
        "opencode existence",
        PASS,
        expected_found_detail(
            "at least one opencode executable on PATH",
            [f"candidate={path}" for path in launchers],
        ),
        "the case can launch the first PATH opencode entrypoint with real credentials",
    )
    entry = launchers[0]
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
        run_direct_opencode_case(env, result, entry)
    except StepFailure:
        return result
    if any(check.status == FAIL for check in result.checks):
        result.status = FAIL
    return result
