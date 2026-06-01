#!/usr/bin/env python3
"""Agent workload for fanotify permission-enforcement E2E."""

import argparse
import os
import sys


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--allowed-path", required=True)
    parser.add_argument("--denied-path", required=True)
    return parser.parse_args()


def read_text(path: str) -> str:
    with open(path, "r", encoding="utf-8") as handle:
        return handle.read()


def main() -> int:
    args = parse_args()
    print(f"agent_pid={os.getpid()}", flush=True)
    control = sys.stdin.readline()
    if control.strip() != "go":
        print(f"unexpected_control={control!r}", flush=True)
        return 2

    read_text(args.allowed_path)
    print("allowed=ok", flush=True)
    try:
        read_text(args.denied_path)
    except PermissionError:
        print("denied=permission_denied", flush=True)
        return 0

    print("denied=unexpected_success", flush=True)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
