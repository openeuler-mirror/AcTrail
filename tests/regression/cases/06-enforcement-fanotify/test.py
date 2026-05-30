"""Fanotify enforcement regression case."""

from __future__ import annotations

from evidence import expected_found_detail, output_line
from model import FAIL, PASS, SKIP, CaseResult


CASE_ID = "enforcement-fanotify"
TITLE = "File access control with fanotify"
SUITES = {"quick", "enforcement", "full"}


def run(env) -> CaseResult:
    result = CaseResult(CASE_ID, TITLE, PASS, 0.0)
    if not env.is_root():
        result.status = SKIP
        result.add_check(
            "root privileges",
            SKIP,
            expected_found_detail("uid=0", ["root privileges missing"]),
            "fanotify permission mode cannot be tested without root",
        )
        return result
    result.add_check(
        "root privileges",
        PASS,
        expected_found_detail("uid=0", ["uid=0"]),
        "fanotify permission events can be installed",
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
    env.output_dir.mkdir(parents=True, exist_ok=True)
    otel_output = str(env.output_dir / "fanotify.otlp.json")
    command = [
        env.python,
        str(env.repo_root / "docs/examples/04.fanotify-enforcement-e2e/run_e2e.py"),
        "--bin-dir",
        str(env.bin_dir),
        "--otel-output",
        otel_output,
    ]
    completed = env.run(command)
    result.command = completed.command
    result.stdout_tail = env.output_tail(completed.stdout)
    result.stderr_tail = env.output_tail(completed.stderr)
    result.report_paths.append(otel_output)
    if completed.returncode != 0:
        result.status = FAIL
        result.add_check(
            "fanotify E2E command",
            FAIL,
            expected_found_detail("docs example exits 0", [f"exit={completed.returncode}"]),
            "script did not complete; stdout/stderr tails show setup, workload, or export failure",
        )
        return result
    result.add_check(
        "fanotify E2E command",
        PASS,
        expected_found_detail("docs example exits 0", [f"exit={completed.returncode}", f"otel_output={otel_output}"]),
        "workload ran under AcTrail and exported enforcement OTEL evidence",
    )
    block_fragments = ("allowed=ok", "denied=permission_denied")
    result.add_check(
        "client-side block proof",
        PASS if all(fragment in completed.output for fragment in block_fragments) else FAIL,
        expected_found_detail(
            "allowed read succeeds and denied read returns permission denied",
            [
                output_line(completed.output, "allowed=ok"),
                output_line(completed.output, "denied=permission_denied"),
            ],
        ),
        "the monitored process read the allowed file and received permission denied for the blocked file",
    )
    span_line = output_line(completed.output, "otel_enforcement_spans=")
    result.add_check(
        "OTEL enforcement spans",
        PASS if "otel_enforcement_spans=allow,deny" in completed.output else FAIL,
        expected_found_detail("OTEL export contains allow and deny enforcement spans", [span_line]),
        "exported OTEL contains both allow and deny enforcement decision spans",
    )
    if any(check.status == FAIL for check in result.checks):
        result.status = FAIL
    return result
