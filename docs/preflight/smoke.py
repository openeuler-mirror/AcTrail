"""Runtime smoke checks that execute the documented local examples."""

from __future__ import annotations

import re
import sys
from pathlib import Path

from .common import FAIL, PASS, WARN, Check, failed_command, run_command


TRACE_RE = re.compile(r"trace trace-(\d+) entered Active")


def runtime_smoke_checks(bin_dir: Path, run_smoke: bool, verbose: bool) -> list[Check]:
    if not run_smoke:
        return [
            Check(
                "eBPF live attach",
                WARN,
                "skipped; run python3 docs/preflight/platform_preflight.py --run-smoke",
            ),
            Check(
                "TLS seccomp user-read",
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
        ebpf_smoke(bin_dir, verbose),
        tls_seccomp_smoke(bin_dir, verbose),
        fanotify_smoke(verbose),
    ]


def ebpf_smoke(bin_dir: Path, verbose: bool) -> Check:
    clean = run_command((sys.executable, "docs/examples/clean.py", "--example", "extended-observation"))
    if clean.returncode != 0:
        return failed_command("eBPF live attach", clean, verbose)
    command = (
        str(bin_dir / "ebpf_probe"),
        "verify-live",
        "--config",
        "docs/examples/03.extended-observation-e2e/observation.conf",
    )
    result = run_command(command)
    if result.returncode != 0:
        return failed_command("eBPF live attach", result, verbose)
    if "live verification passed" not in result.stdout:
        return Check("eBPF live attach", FAIL, "verify-live did not print live verification passed")
    return Check("eBPF live attach", PASS, "verify-live completed")


def tls_seccomp_smoke(bin_dir: Path, verbose: bool) -> Check:
    config = "docs/examples/02.llm-http-payload-capture/http2-local/operator.conf"
    clean = run_command((sys.executable, "docs/examples/clean.py", "--example", "http2-local"))
    if clean.returncode != 0:
        return failed_command("TLS seccomp user-read", clean, verbose)
    start = run_command((str(bin_dir / "actraild"), "start", "--config", config))
    if start.returncode != 0:
        return failed_command("TLS seccomp user-read", start, verbose)
    try:
        launch = run_command(
            (
                str(bin_dir / "actrailctl"),
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
            return failed_command("TLS seccomp user-read", launch, verbose)
        trace_id = parse_trace_id(launch.stdout)
        if trace_id is None:
            return Check("TLS seccomp user-read", FAIL, "actrailctl launch did not print a trace id")
        payloads = run_command(
            (
                str(bin_dir / "actrailviewer"),
                "payloads",
                "--config",
                config,
                "--trace-id",
                str(trace_id),
            )
        )
        if payloads.returncode != 0:
            return failed_command("TLS seccomp user-read", payloads, verbose)
        events = run_command(
            (
                str(bin_dir / "actrailviewer"),
                "events",
                "--config",
                config,
                "--trace-id",
                str(trace_id),
            )
        )
        if events.returncode != 0:
            return failed_command("TLS seccomp user-read", events, verbose)
    finally:
        run_command((str(bin_dir / "actraild"), "stop", "--config", config))
    payload_ok = all(
        value in payloads.stdout for value in ("TlsUserSpace", "openssl", "Complete", "success")
    )
    event_ok = "Application" in events.stdout and ("frame" in events.stdout or "data" in events.stdout)
    if not payload_ok:
        return Check("TLS seccomp user-read", FAIL, "payload output missed complete OpenSSL rows")
    if not event_ok:
        return Check("TLS seccomp user-read", FAIL, "event output missed HTTP/2 Application rows")
    return Check("TLS seccomp user-read", PASS, f"trace-{trace_id} has complete TLS payload rows")


def fanotify_smoke(verbose: bool) -> Check:
    result = run_command((sys.executable, "docs/examples/04.fanotify-enforcement-e2e/run_e2e.py"))
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
