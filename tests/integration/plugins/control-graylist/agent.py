#!/usr/bin/env python3
"""Agent workload for WASM ControlDecider graylist E2E."""

import argparse
import os
import sys


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--allowed-path", required=True)
    parser.add_argument("--gray-path", required=True)
    parser.add_argument("--denied-path", required=True)
    parser.add_argument("--gray-repeat", type=int, default=1)
    parser.add_argument("--expect-gray-denied", action="store_true")
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
    for index in range(args.gray_repeat):
        label = "gray" if index == 0 else f"gray{index + 1}"
        try:
            read_text(args.gray_path)
        except PermissionError:
            print(f"{label}=permission_denied", flush=True)
            if args.expect_gray_denied:
                continue
            return 1
        if args.expect_gray_denied:
            print(f"{label}=unexpected_success", flush=True)
            return 1
        print(f"{label}=ok", flush=True)
    try:
        read_text(args.denied_path)
    except PermissionError:
        print("denied=permission_denied", flush=True)
        return 0

    print("denied=unexpected_success", flush=True)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
