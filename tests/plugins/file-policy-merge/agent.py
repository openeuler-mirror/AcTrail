#!/usr/bin/env python3
"""Agent workload for file policy merge E2E."""

import argparse
import os
import sys


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--step", action="append", required=True)
    return parser.parse_args()


def read_text(path: str) -> None:
    with open(path, "r", encoding="utf-8") as handle:
        handle.read()


def main() -> int:
    args = parse_args()
    print(f"agent_pid={os.getpid()}", flush=True)
    control = sys.stdin.readline()
    if control.strip() != "go":
        print(f"unexpected_control={control!r}", flush=True)
        return 2

    for raw_step in args.step:
        fields = raw_step.split(":", 2)
        if len(fields) != 3:
            print(f"invalid_step={raw_step}", flush=True)
            return 2
        label, expected, path = fields
        try:
            read_text(path)
            actual = "ok"
        except PermissionError:
            actual = "permission_denied"
        print(f"{label}={actual}", flush=True)
        if actual != expected:
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
