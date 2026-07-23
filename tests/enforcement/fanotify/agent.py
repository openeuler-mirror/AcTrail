#!/usr/bin/env python3
"""Agent workload for fanotify permission-enforcement E2E."""

import argparse
import os
import shutil
import subprocess
import sys


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--allowed-path", required=True)
    parser.add_argument("--denied-path", required=True)
    parser.add_argument("--pre-exec-denied-path", required=True)
    return parser.parse_args()


def read_text(path: str) -> str:
    with open(path, "r", encoding="utf-8") as handle:
        return handle.read()


def verify_pre_exec_redirection_is_denied(path: str) -> bool:
    base64 = shutil.which("base64")
    if base64 is None:
        print("pre_exec_redirection=base64_missing", flush=True)
        return False
    command = (
        '"$2" < "$1" >/dev/null & child=$!; '
        'wait "$child"; status=$?; exit "$status"'
    )
    completed = subprocess.run(
        ["/bin/sh", "-c", command, "sh", path, base64],
        text=True,
        capture_output=True,
        check=False,
        env={**os.environ, "LC_ALL": "C"},
    )
    if completed.returncode == 0:
        print("pre_exec_redirection=unexpected_success", flush=True)
        return False
    if not any(
        message in completed.stderr
        for message in ("Permission denied", "Operation not permitted")
    ):
        print("pre_exec_redirection=unexpected_failure", flush=True)
        return False
    print("pre_exec_redirection=permission_denied", flush=True)
    return True


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
        return 0 if verify_pre_exec_redirection_is_denied(args.pre_exec_denied_path) else 1

    print("denied=unexpected_success", flush=True)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
