"""Plain HTTP socket payload regression case."""

from __future__ import annotations

from evidence import expected_found_detail, line_containing
from model import FAIL, PASS, SKIP, CaseResult


CASE_ID = "http-payload"
TITLE = "Plain HTTP socket payload and HTTP semantics"
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
    command = [
        env.python,
        str(env.repo_root / "docs/examples/05.http-payload-unified/run_e2e.py"),
        "--bin-dir",
        str(env.bin_dir),
    ]
    completed = env.run(command)
    result.command = completed.command
    result.stdout_tail = env.output_tail(completed.stdout)
    result.stderr_tail = env.output_tail(completed.stderr)
    if completed.returncode != 0:
        result.status = FAIL
        result.add_check(
            "HTTP payload E2E command",
            FAIL,
            expected_found_detail("docs example exits 0", [f"exit={completed.returncode}"]),
            "script did not complete; stdout/stderr tails show workload, capture, or viewer failure",
        )
        return result
    result.add_check(
        "HTTP payload E2E command",
        PASS,
        expected_found_detail("docs example exits 0", [f"exit={completed.returncode}"]),
        "local HTTP workload completed under AcTrail",
    )
    request_fragments = ("Application", "request", "POST /plain-http")
    result.add_check(
        "HTTP request semantics",
        PASS
        if all(fragment in completed.output for fragment in request_fragments)
        else FAIL,
        expected_found_detail(
            "Application request event for POST /plain-http",
            [
                line_containing(completed.output, request_fragments),
            ],
        ),
        "viewer reported the plaintext HTTP request as an application event",
    )
    result.add_check(
        "socket payload source",
        PASS if "Syscall" in completed.output else FAIL,
        expected_found_detail(
            "payload source includes Syscall/socket-syscall",
            [
                line_containing(completed.output, ("Syscall", "socket-syscall")),
            ],
        ),
        "viewer payload rows came from the plain socket syscall boundary",
    )
    if any(check.status == FAIL for check in result.checks):
        result.status = FAIL
    return result
