"""Process helpers for runtime verification."""

from __future__ import annotations

import os
import signal
import subprocess
from dataclasses import dataclass


@dataclass(frozen=True)
class TargetRun:
    returncode: int | None
    timed_out: bool
    stdout: str
    stderr: str


def command_after_delimiter(raw: list[str]) -> list[str]:
    if not raw:
        raise RuntimeError("trace target command must follow binary and --")
    command = raw[1:] if raw[0] == "--" else raw
    if not command:
        raise RuntimeError("missing trace target command")
    return command


def run_target_for_duration(command: list[str], seconds: float) -> TargetRun:
    process = subprocess.Popen(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        start_new_session=True,
    )
    timed_out = False
    try:
        stdout, stderr = process.communicate(timeout=seconds)
    except subprocess.TimeoutExpired:
        timed_out = True
        os.killpg(process.pid, signal.SIGTERM)
        stdout, stderr = process.communicate()
    return TargetRun(process.returncode, timed_out, stdout, stderr)


def print_target_result(result: TargetRun) -> None:
    print(f"target_returncode={result.returncode}")
    print(f"target_timed_out={str(result.timed_out).lower()}")
    print("--- target stdout ---")
    print(result.stdout, end="")
    print("--- target stderr ---")
    print(result.stderr, end="")
