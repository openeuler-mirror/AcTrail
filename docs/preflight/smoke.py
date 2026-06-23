"""Runtime smoke checks that execute the documented local examples."""

from __future__ import annotations

import re
import sys
from pathlib import Path

from .checks import ResolvedArtifacts
from .common import (
    FAIL,
    PASS,
    WARN,
    Check,
    CommandResult,
    failed_command,
    print_command_failure,
    run_command,
)


TRACE_RE = re.compile(r"trace trace-(\d+) entered Active")
EBPF_LIBBPF_DEBUG_ENV = "ACTRAIL_EBPF_LIBBPF_DEBUG"
EBPF_DEBUG_LINE_LIMIT = 12
EBPF_DEBUG_CHAR_LIMIT = 1800
EBPF_LOAD_LOG_DELIMITERS = (
    "-- begin prog load log --",
    "-- end prog load log --",
)
EBPF_NON_FATAL_LIBBPF_MARKERS = ("skipping optional step",)
EBPF_VERIFIER_MARKERS = (
    "bitwise operator",
    "invalid access",
    "invalid mem",
    "pointer prohibited",
    "prohibited",
    "reg type unsupported",
    "unbounded",
)


def runtime_smoke_checks(artifacts: ResolvedArtifacts, run_smoke: bool, verbose: bool) -> list[Check]:
    if not run_smoke:
        return [
            Check(
                "eBPF live attach",
                WARN,
                "skipped; run python3 docs/preflight/platform_preflight.py --run-smoke",
            ),
            Check(
                "TLS sync payload",
                WARN,
                "skipped; run python3 docs/preflight/platform_preflight.py --run-smoke",
            ),
            Check(
                "fanotify permission enforcement",
                WARN,
                "skipped; run python3 docs/preflight/platform_preflight.py --run-smoke",
            ),
            Check(
                "process seccomp agent invocation",
                WARN,
                "skipped; run python3 tests/process/agent-invocation/run_e2e.py",
                required=False,
            ),
        ]
    return [
        ebpf_smoke(artifacts, verbose),
        tls_sync_smoke(artifacts, verbose),
        fanotify_smoke(artifacts, verbose),
    ]


def ebpf_smoke(artifacts: ResolvedArtifacts, verbose: bool) -> Check:
    ebpf_probe = artifacts.path("ebpf_probe")
    actrailctl = artifacts.path("actrailctl")
    if ebpf_probe is None or actrailctl is None:
        return Check(
            "eBPF live attach",
            FAIL,
            missing_detail(artifacts, ("ebpf_probe", "actrailctl")),
        )
    clean = run_command(
        (
            sys.executable,
            "docs/examples/clean.py",
            "--bin-dir",
            str(actrailctl.parent),
            "--example",
            "extended-observation",
        )
    )
    if clean.returncode != 0:
        return failed_command("eBPF live attach", clean, verbose)
    command = (
        str(ebpf_probe),
        "verify-live",
        "--config",
        "docs/examples/03.extended-observation-e2e/observation.conf",
    )
    result = run_command(command, env={EBPF_LIBBPF_DEBUG_ENV: "1"})
    if result.returncode != 0:
        return failed_ebpf_command(ebpf_probe, result, verbose)
    if "live verification passed" not in result.stdout:
        return Check("eBPF live attach", FAIL, "verify-live did not print live verification passed")
    return Check("eBPF live attach", PASS, "verify-live completed")


def tls_sync_smoke(artifacts: ResolvedArtifacts, verbose: bool) -> Check:
    required = {
        "actraild": artifacts.path("actraild"),
        "actrailctl": artifacts.path("actrailctl"),
        "actrailviewer": artifacts.path("actrailviewer"),
    }
    missing = [f"{name}: {artifacts.detail(name)}" for name, path in required.items() if path is None]
    if missing:
        return Check("TLS sync payload", FAIL, "; ".join(missing))
    actraild = required["actraild"]
    actrailctl = required["actrailctl"]
    actrailviewer = required["actrailviewer"]
    assert actraild is not None
    assert actrailctl is not None
    assert actrailviewer is not None
    config = "docs/examples/02.llm-http-payload-capture/http2-local/operator.conf"
    clean = run_command(
        (
            sys.executable,
            "docs/examples/clean.py",
            "--bin-dir",
            str(actrailctl.parent),
            "--example",
            "http2-local",
        )
    )
    if clean.returncode != 0:
        return failed_command("TLS sync payload", clean, verbose)
    start = run_command((str(actraild), "--config", config, "start"))
    if start.returncode != 0:
        return failed_command("TLS sync payload", start, verbose)
    try:
        launch = run_command(
            (
                str(actrailctl),
                "launch",
                "--config",
                config,
                "--name",
                "http2-local-platform-preflight",
                "--",
                sys.executable,
                "docs/examples/02.llm-http-payload-capture/http2-local/workload.py",
                "--target-config",
                "docs/examples/02.llm-http-payload-capture/http2-local/workload.conf",
            )
        )
        if launch.returncode != 0:
            return failed_command("TLS sync payload", launch, verbose)
        trace_id = parse_trace_id(launch.stdout)
        if trace_id is None:
            return Check("TLS sync payload", FAIL, "actrailctl launch did not print a trace id")
        payloads = run_command(
            (
                str(actrailviewer),
                "payloads",
                "--config",
                config,
                "--trace-id",
                str(trace_id),
            )
        )
        if payloads.returncode != 0:
            return failed_command("TLS sync payload", payloads, verbose)
        events = run_command(
            (
                str(actrailviewer),
                "events",
                "--config",
                config,
                "--trace-id",
                str(trace_id),
            )
        )
        if events.returncode != 0:
            return failed_command("TLS sync payload", events, verbose)
    finally:
        run_command((str(actraild), "--config", config, "stop"))
    payload_ok = all(
        value in payloads.stdout for value in ("TlsUserSpace", "openssl", "Complete", "success")
    )
    event_ok = "Application" in events.stdout and ("frame" in events.stdout or "data" in events.stdout)
    if not payload_ok:
        return Check("TLS sync payload", FAIL, "payload output missed complete OpenSSL rows")
    if not event_ok:
        return Check("TLS sync payload", FAIL, "event output missed HTTP/2 Application rows")
    return Check("TLS sync payload", PASS, f"trace-{trace_id} has complete TLS payload rows")


def fanotify_smoke(artifacts: ResolvedArtifacts, verbose: bool) -> Check:
    bin_dir = common_binary_dir(artifacts, ("actraild", "actrailctl", "actrailviewer"))
    if bin_dir is None:
        return Check(
            "fanotify permission enforcement",
            FAIL,
            "fanotify e2e requires co-located actraild, actrailctl, and actrailviewer; "
            + missing_detail(artifacts, ("actraild", "actrailctl", "actrailviewer")),
        )
    result = run_command(
        (
            sys.executable,
            "docs/examples/04.fanotify-enforcement-e2e/run_e2e.py",
            "--bin-dir",
            str(bin_dir),
        )
    )
    if result.returncode != 0:
        return failed_command("fanotify permission enforcement", result, verbose)
    expected = ("allowed=ok", "denied=permission_denied", "decision=allow", "decision=deny")
    missing = [value for value in expected if value not in result.stdout]
    if missing:
        return Check("fanotify permission enforcement", FAIL, f"missing {', '.join(missing)}")
    return Check("fanotify permission enforcement", PASS, "agent allow/deny and AcTrail decisions matched")


def parse_trace_id(output: str) -> int | None:
    match = TRACE_RE.search(output)
    return int(match.group(1)) if match else None


def failed_ebpf_command(ebpf_probe: Path, result: CommandResult, verbose: bool) -> Check:
    if verbose:
        print_command_failure(result)
    detail = summarize_ebpf_debug_output(result)
    return Check("eBPF live attach", FAIL, f"{ebpf_probe}: {detail}")


def summarize_ebpf_debug_output(result: CommandResult) -> str:
    combined = "\n".join(part for part in (result.stderr, result.stdout) if part)
    lines = [line.strip() for line in combined.splitlines() if line.strip()]
    if not lines:
        return f"exit={result.returncode}"
    primary = primary_ebpf_error_line(lines)
    error_lines = [line for line in lines if is_ebpf_error_line(line)]
    if error_lines:
        debug = bounded_error_excerpt(error_lines, primary)
        if primary is not None and primary not in error_lines:
            return f"{bounded_text(primary)}; libbpf/verifier: {debug}"
        return f"libbpf/verifier: {debug}"
    return bounded_text(primary or lines[-1])


def primary_ebpf_error_line(lines: list[str]) -> str | None:
    for line in reversed(lines):
        if is_verifier_diagnostic_line(line):
            return line
    for line in reversed(lines):
        if is_command_error_line(line):
            return line
    return last_meaningful_line(lines)


def last_meaningful_line(lines: list[str]) -> str | None:
    for line in reversed(lines):
        if not is_ebpf_noise_line(line):
            return line
    return None


def is_ebpf_error_line(line: str) -> bool:
    lowered = line.lower()
    if is_ebpf_noise_line(line):
        return False
    if is_verifier_diagnostic_line(line) or is_command_error_line(line):
        return True
    if lowered.startswith("processed "):
        return True
    if "libbpf:" not in lowered:
        return False
    return any(
        marker in lowered
        for marker in (
            "error",
            "failed",
            "denied",
            "invalid",
            "permission",
            "processed ",
        )
    )


def is_verifier_diagnostic_line(line: str) -> bool:
    lowered = line.lower()
    if is_ebpf_noise_line(line):
        return False
    return any(marker in lowered for marker in EBPF_VERIFIER_MARKERS)


def is_command_error_line(line: str) -> bool:
    lowered = line.lower()
    if is_ebpf_noise_line(line):
        return False
    return any(
        marker in lowered
        for marker in (
            "bpf program load failed",
            "control command failed",
            "failed to load",
            "failed to load object",
            "load_object",
            "missing file operations",
            "permission denied",
        )
    )


def is_ebpf_noise_line(line: str) -> bool:
    lowered = line.lower()
    return any(marker in lowered for marker in EBPF_LOAD_LOG_DELIMITERS) or any(
        marker in lowered for marker in EBPF_NON_FATAL_LIBBPF_MARKERS
    )


def bounded_error_excerpt(lines: list[str], primary: str | None) -> str:
    tail = lines[-EBPF_DEBUG_LINE_LIMIT:]
    if primary is not None and primary in lines and primary not in tail:
        return bounded_text(f"{primary} | ... " + " | ".join(tail))
    text = " | ".join(tail)
    if len(lines) > EBPF_DEBUG_LINE_LIMIT:
        text = "... " + text
    return bounded_text(text)


def bounded_text(text: str) -> str:
    if len(text) > EBPF_DEBUG_CHAR_LIMIT:
        text = "... " + text[-EBPF_DEBUG_CHAR_LIMIT:]
    return text


def common_binary_dir(artifacts: ResolvedArtifacts, names: tuple[str, ...]) -> Path | None:
    paths = [artifacts.path(name) for name in names]
    if any(path is None for path in paths):
        return None
    parents = {path.parent for path in paths if path is not None}
    if len(parents) != 1:
        return None
    return next(iter(parents))


def missing_detail(artifacts: ResolvedArtifacts, names: tuple[str, ...]) -> str:
    return "; ".join(f"{name}: {artifacts.detail(name)}" for name in names)
