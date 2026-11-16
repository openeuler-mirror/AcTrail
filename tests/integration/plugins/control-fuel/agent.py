#!/usr/bin/env python3
"""Agent workload for WASM ControlDecider fuel exhaustion E2E."""

import argparse
import os
import sys


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--gray-path", required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    print(f"agent_pid={os.getpid()}", flush=True)
    control = sys.stdin.readline()
    if control.strip() != "go":
        print(f"unexpected_control={control!r}", flush=True)
        return 2
    try:
        with open(args.gray_path, "r", encoding="utf-8") as handle:
            handle.read()
    except PermissionError:
        print("gray=permission_denied", flush=True)
        return 0

    print("gray=unexpected_success", flush=True)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
