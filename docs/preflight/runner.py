"""CLI runner for the AcTrail platform preflight."""

from __future__ import annotations

import argparse
import fcntl
import os
from pathlib import Path

from .checks import (
    ResolvedArtifacts,
    agent_tls_checks,
    kernel_checks,
    platform_checks,
    release_artifact_checks,
    resolve_release_artifacts,
    shared_openssl_checks,
    tool_checks,
)
from .common import FAIL, PASS, WARN, Check, Color, format_check
from .smoke import runtime_smoke_checks


DEFAULT_SMOKE_LOCK_PATH = "/tmp/actrail-platform-preflight-smoke.lock"
DEFAULT_EBPF_SMOKE_CONFIG = "docs/examples/03.extended-observation-e2e/observation.conf"


def main() -> int:
    args = parse_args()
    color = Color(args.color)
    artifacts = resolve_release_artifacts(args.bin_dir)
    sections: list[tuple[str, list[Check]]] = [
        ("Platform", platform_checks()),
        ("Release Artifacts", release_artifact_checks(artifacts)),
        ("Kernel Interfaces", kernel_checks()),
        ("Build And Example Tools", tool_checks()),
        ("Shared OpenSSL", shared_openssl_checks()),
        ("Agent Executable TLS", agent_tls_checks()),
        (
            "Runtime Smoke",
            locked_runtime_smoke_checks(
                artifacts,
                args.run_smoke,
                args.verbose,
                Path(args.smoke_lock_path),
                args.ebpf_smoke_config,
            ),
        ),
    ]
    print("AcTrail platform preflight")
    print()
    for title, checks in sections:
        print(title)
        for check in checks:
            print(format_check(check, color))
        print()
    return summarize(sections, color)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Print AcTrail platform readiness checks.")
    parser.add_argument(
        "--bin-dir",
        default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"),
        help=(
            "release artifact directory, or a path to one release artifact; "
            "defaults to ACTRAIL_BIN_DIR or target/release with PATH lookup for missing executables"
        ),
    )
    parser.add_argument(
        "--run-smoke",
        action="store_true",
        help="run local eBPF, TLS seccomp, and fanotify transfer smoke checks",
    )
    parser.add_argument(
        "--color",
        choices=("auto", "always", "never"),
        default="auto",
        help="colorize status symbols",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="print captured stdout/stderr when a runtime smoke command fails",
    )
    parser.add_argument(
        "--ebpf-smoke-config",
        default=os.environ.get("ACTRAIL_PREFLIGHT_EBPF_CONFIG", DEFAULT_EBPF_SMOKE_CONFIG),
        help=(
            "verify-live config for the eBPF smoke; defaults to "
            "ACTRAIL_PREFLIGHT_EBPF_CONFIG or " + DEFAULT_EBPF_SMOKE_CONFIG
        ),
    )
    parser.add_argument(
        "--smoke-lock-path",
        default=os.environ.get("ACTRAIL_PREFLIGHT_SMOKE_LOCK_PATH", DEFAULT_SMOKE_LOCK_PATH),
        help=(
            "exclusive lock file for --run-smoke; defaults to "
            "ACTRAIL_PREFLIGHT_SMOKE_LOCK_PATH or " + DEFAULT_SMOKE_LOCK_PATH
        ),
    )
    return parser.parse_args()


def locked_runtime_smoke_checks(
    artifacts: ResolvedArtifacts,
    run_smoke: bool,
    verbose: bool,
    lock_path: Path,
    ebpf_config: str,
) -> list[Check]:
    if not run_smoke:
        return runtime_smoke_checks(artifacts, run_smoke, verbose, ebpf_config)
    flags = os.O_CREAT | os.O_RDWR | os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(lock_path, flags, 0o600)
    except OSError as error:
        return [Check("runtime smoke isolation", FAIL, f"open lock {lock_path}: {error}")]
    try:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            os.lseek(descriptor, 0, os.SEEK_SET)
            owner = os.read(descriptor, 256).decode("utf-8", errors="replace").strip()
            detail = f"another --run-smoke owns {lock_path}"
            if owner:
                detail += f" ({owner})"
            return [Check("runtime smoke isolation", FAIL, detail)]
        owner = f"pid={os.getpid()} cwd={Path.cwd()}"
        os.ftruncate(descriptor, 0)
        os.write(descriptor, owner.encode("utf-8"))
        os.fsync(descriptor)
        return runtime_smoke_checks(artifacts, run_smoke, verbose, ebpf_config)
    finally:
        os.close(descriptor)


def summarize(sections: list[tuple[str, list[Check]]], color: Color) -> int:
    checks = [check for _, values in sections for check in values]
    blocking = [check for check in checks if check.required and check.status == FAIL]
    warnings = [check for check in checks if check.status == WARN]
    optional_failures = [check for check in checks if not check.required and check.status == FAIL]
    if blocking:
        print(color.status(FAIL, f"Summary: {len(blocking)} blocking failure(s)"))
        return 1
    print(
        color.status(
            PASS,
            "Summary: no blocking failures; "
            f"{len(warnings)} warning(s), {len(optional_failures)} optional failure(s)",
        )
    )
    return 0
