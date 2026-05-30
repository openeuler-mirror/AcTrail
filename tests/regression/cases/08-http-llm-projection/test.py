"""HTTP socket payload projection to llm.request regression case."""

from __future__ import annotations

from pathlib import Path

from e2e_steps.checks import StepFailure
from e2e_steps.loader import load_package
from evidence import expected_found_detail
from model import FAIL, PASS, SKIP, CaseResult

CASE_DIR = Path(__file__).resolve().parent
DIRECT_STEPS = load_package("regression_http_llm_projection_direct_steps", CASE_DIR / "direct_steps")
run_direct_http_projection_case = DIRECT_STEPS.run_direct_http_projection_case


CASE_ID = "http-llm-projection"
TITLE = "Plain HTTP socket payload projected as llm.request"
SUITES = {"quick", "payload", "full"}


def run(env) -> CaseResult:
    result = CaseResult(CASE_ID, TITLE, PASS, 0.0)
    if not env.is_root():
        result.status = SKIP
        result.add_check(
            "root privileges",
            SKIP,
            expected_found_detail("uid=0", ["root privileges missing"]),
            "socket payload probes cannot be loaded without root/eBPF privileges",
        )
        return result
    result.add_check(
        "root privileges",
        PASS,
        expected_found_detail("uid=0", ["uid=0"]),
        "socket payload probes can be loaded",
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
        run_direct_http_projection_case(env, result)
    except StepFailure:
        return result
    if any(check.status == FAIL for check in result.checks):
        result.status = FAIL
    return result
