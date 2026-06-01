"""CLI runner for the AcTrail platform preflight."""

from __future__ import annotations

import argparse
import os
from pathlib import Path

from .checks import (
    agent_tls_checks,
    kernel_checks,
    platform_checks,
    release_binary_checks,
    shared_openssl_checks,
    tool_checks,
)
from .common import FAIL, PASS, WARN, Check, Color, format_check
from .smoke import runtime_smoke_checks


def main() -> int:
    args = parse_args()
    color = Color(args.color)
    bin_dir = Path.cwd() / args.bin_dir
    sections: list[tuple[str, list[Check]]] = [
        ("Platform", platform_checks()),
        ("Release Binaries", release_binary_checks(bin_dir)),
        ("Kernel Interfaces", kernel_checks()),
        ("Build And Example Tools", tool_checks()),
        ("Shared OpenSSL", shared_openssl_checks()),
        ("Agent Executable TLS", agent_tls_checks()),
        ("Runtime Smoke", runtime_smoke_checks(bin_dir, args.run_smoke, args.verbose)),
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
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
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
    return parser.parse_args()


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
