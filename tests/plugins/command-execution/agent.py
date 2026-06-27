#!/usr/bin/env python3
"""Agent workload for command-execution ControlDecider E2E."""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--allowed-command", required=True)
    parser.add_argument("--denied-command", required=True)
    return parser.parse_args()


def run_allowed(path: Path) -> None:
    result = subprocess.run([str(path)], text=True, capture_output=True, check=False)
    if result.returncode != 0:
        raise RuntimeError(
            f"allowed command failed rc={result.returncode} stdout={result.stdout} stderr={result.stderr}"
        )
    print("allowed=ok", flush=True)


def run_denied(path: Path) -> None:
    try:
        result = subprocess.run([str(path)], text=True, capture_output=True, check=False)
    except PermissionError:
        print("denied=permission_denied", flush=True)
        return
    if result.returncode == 0:
        raise RuntimeError(f"denied command unexpectedly succeeded stdout={result.stdout}")
    combined = result.stdout + result.stderr
    if "Permission denied" not in combined and "Operation not permitted" not in combined:
        raise RuntimeError(
            f"denied command failed for unexpected reason rc={result.returncode} output={combined}"
        )
    print("denied=permission_denied", flush=True)


def main() -> int:
    args = parse_args()
    print(f"agent_pid={os.getpid()}", flush=True)
    control = sys.stdin.readline()
    if control.strip() != "go":
        print(f"unexpected_control={control!r}", flush=True)
        return 2
    run_allowed(Path(args.allowed_command))
    run_denied(Path(args.denied_command))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
