"""CLI runner for the AcTrail platform preflight."""

from __future__ import annotations

import argparse
import os

from .checks import (
    kernel_checks,
    platform_checks,
    release_artifact_checks,
    resolve_release_artifacts,
    shared_openssl_checks,
    tool_checks,
)
from .common import FAIL, PASS, WARN, Check, Color, format_check


def main() -> int:
    args = parse_args()
    color = Color(args.color)
    artifacts = resolve_release_artifacts(args.bin_dir)
    sections: list[tuple[str, list[Check]]] = [
        ("Platform", platform_checks()),
        ("Release Artifacts", release_artifact_checks(artifacts)),
        ("Kernel Interfaces", kernel_checks()),
        ("Build And Runtime Tools", tool_checks()),
        ("Shared OpenSSL", shared_openssl_checks()),
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
        "--color",
        choices=("auto", "always", "never"),
        default="auto",
        help="colorize status symbols",
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
