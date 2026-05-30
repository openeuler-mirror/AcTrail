"""Regression result model."""

from __future__ import annotations

import sys
from dataclasses import dataclass, field


PASS = "pass"
FAIL = "fail"
SKIP = "skip"
WARN = "warn"


STATUS_COLORS = {
    PASS: "\033[32m",
    FAIL: "\033[31m",
    SKIP: "\033[36m",
    WARN: "\033[33m",
    "reset": "\033[0m",
}
SUMMARY_STATUSES = (PASS, SKIP, FAIL)
OUTPUT_COLOR_MODE = "auto"


def set_color_mode(color_mode: str) -> None:
    global OUTPUT_COLOR_MODE

    OUTPUT_COLOR_MODE = color_mode


@dataclass
class CheckResult:
    name: str
    status: str
    detail: str = ""
    evidence: str = ""


@dataclass
class CaseResult:
    case_id: str
    title: str
    status: str
    duration_seconds: float
    checks: list[CheckResult] = field(default_factory=list)
    command: list[str] = field(default_factory=list)
    stdout_tail: str = ""
    stderr_tail: str = ""
    report_paths: list[str] = field(default_factory=list)

    def begin_check(self, name: str, detail: str = "running") -> None:
        suffix = f": {detail}" if detail else ""
        print(f"    ... {name}{suffix}", flush=True)

    def add_check(self, name: str, status: str, detail: str = "", evidence: str = "") -> None:
        check = CheckResult(name, status, detail, evidence)
        self.checks.append(check)
        item_index = len(self.checks)
        suffix = detail_suffix(detail)
        print(f"    - {status_label(status)} check {item_index} {name}{suffix}", flush=True)
        if evidence and status == FAIL:
            print(f"        reason: {evidence}", flush=True)


def status_label(status: str) -> str:
    label = status.upper()
    if not should_color():
        return label
    return f"{STATUS_COLORS.get(status, '')}{label}{STATUS_COLORS['reset']}"


def should_color() -> bool:
    if OUTPUT_COLOR_MODE == "never":
        return False
    if OUTPUT_COLOR_MODE == "auto" and not sys.stdout.isatty():
        return False
    return True


def detail_suffix(detail: str) -> str:
    if not detail:
        return ""
    if detail.startswith("\n"):
        return f":{detail}"
    return f": {detail}"


def check_status_counts(result: CaseResult) -> dict[str, int]:
    counts = {status: 0 for status in (*SUMMARY_STATUSES, WARN)}
    for check in result.checks:
        counts[check.status] = counts.get(check.status, 0) + 1
    return counts


def check_status_summary(result: CaseResult) -> str:
    counts = check_status_counts(result)
    parts = [f"{status.upper()}={counts[status]}" for status in SUMMARY_STATUSES]
    if counts[WARN]:
        parts.append(f"{WARN.upper()}={counts[WARN]}")
    for status in sorted(set(counts) - set((*SUMMARY_STATUSES, WARN))):
        if counts[status]:
            parts.append(f"{status.upper()}={counts[status]}")
    return " ".join(parts)


@dataclass
class CommandResult:
    command: list[str]
    returncode: int
    stdout: str
    stderr: str

    @property
    def output(self) -> str:
        return f"{self.stdout}{self.stderr}"
