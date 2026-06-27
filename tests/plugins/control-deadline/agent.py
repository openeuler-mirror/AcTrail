#!/usr/bin/env python3
"""Agent workload for WASM ControlDecider deadline isolation E2E."""

from __future__ import annotations

import argparse
import os
import sys
import time


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--fast-path", required=True)
    parser.add_argument("--slow-path", required=True)
    return parser.parse_args()


def open_marker(path: str) -> str:
    try:
        with open(path, "r", encoding="utf-8") as handle:
            handle.read()
    except PermissionError:
        return "permission_denied"
    return "ok"


def main() -> int:
    args = parse_args()
    print(f"agent_pid={os.getpid()}", flush=True)
    control = sys.stdin.readline()
    if control.strip() != "go":
        print(f"unexpected_control={control!r}", flush=True)
        return 2

    first = open_marker(args.fast_path)
    print(f"first={first}", flush=True)
    started = time.monotonic()
    second = open_marker(args.slow_path)
    elapsed_ms = int((time.monotonic() - started) * 1000)
    print(f"second={second}", flush=True)
    print(f"second_elapsed_ms={elapsed_ms}", flush=True)

    if first != "ok" or second != "permission_denied":
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
