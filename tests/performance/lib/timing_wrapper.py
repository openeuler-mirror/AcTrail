#!/usr/bin/env python3
"""Run one benchmark target and emit target-only runtime."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time


MARKER = "BENCHMARK_TASK_TIMING "


def main() -> int:
    args = parse_args()
    start_ns = time.monotonic_ns()
    result = subprocess.run(args.command, text=True, capture_output=True, check=False)
    task_runtime_ms = (time.monotonic_ns() - start_ns) / 1_000_000
    sys.stdout.write(result.stdout)
    sys.stderr.write(result.stderr)
    print(
        "\n"
        + MARKER
        + json.dumps(
            {
                "task_runtime_ms": task_runtime_ms,
                "exit_code": result.returncode,
            },
            separators=(",", ":"),
        ),
        flush=True,
    )
    return result.returncode


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.command and args.command[0] == "--":
        args.command = args.command[1:]
    if not args.command:
        raise RuntimeError("missing command after --")
    return args


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"benchmark timing wrapper failed: {error}", file=sys.stderr)
        raise SystemExit(1)
