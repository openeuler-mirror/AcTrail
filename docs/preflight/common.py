"""Shared model, command, and formatting helpers."""

from __future__ import annotations

import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


PASS = "pass"
WARN = "warn"
FAIL = "fail"


@dataclass(frozen=True)
class Check:
    name: str
    status: str
    detail: str
    required: bool = True


@dataclass(frozen=True)
class CommandResult:
    command: tuple[str, ...]
    returncode: int
    stdout: str
    stderr: str


class Color:
    def __init__(self, mode: str) -> None:
        self.enabled = mode == "always" or (
            mode == "auto" and sys.stdout.isatty() and "NO_COLOR" not in os.environ
        )

    def status(self, value: str, text: str) -> str:
        if not self.enabled:
            return text
        if value == PASS:
            return f"\033[1;32m{text}\033[0m"
        if value == WARN:
            return f"\033[1;33m{text}\033[0m"
        if value == FAIL:
            return f"\033[1;31m{text}\033[0m"
        return text


def format_check(check: Check, color: Color) -> str:
    symbol = {PASS: "✓", WARN: "!", FAIL: "✗"}[check.status]
    scope = "required" if check.required else "optional"
    return f"  {color.status(check.status, symbol)} {check.name}: {check.detail} [{scope}]"


def run_command(command: tuple[str, ...], env: dict[str, str] | None = None) -> CommandResult:
    command_env = None
    if env is not None:
        command_env = os.environ.copy()
        command_env.update(env)
    result = subprocess.run(command, text=True, capture_output=True, check=False, env=command_env)
    return CommandResult(command, result.returncode, result.stdout, result.stderr)


def failed_command(name: str, result: CommandResult, verbose: bool) -> Check:
    if verbose:
        print_command_failure(result)
    detail = last_line(result.stderr) or last_line(result.stdout) or f"exit={result.returncode}"
    return Check(name, FAIL, detail)


def print_command_failure(result: CommandResult) -> None:
    print(f"Command failed: {' '.join(result.command)}", file=sys.stderr)
    if result.stdout:
        print(result.stdout, file=sys.stderr)
    if result.stderr:
        print(result.stderr, file=sys.stderr)


def last_line(text: str) -> str:
    lines = [line for line in text.splitlines() if line.strip()]
    return lines[-1] if lines else ""


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8", errors="ignore")
    except OSError:
        return ""
