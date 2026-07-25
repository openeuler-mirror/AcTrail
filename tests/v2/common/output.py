from __future__ import annotations

import sys
from dataclasses import dataclass, field
from typing import TextIO

from .test_case import TestResult, TestStatus


_COLORS = {
    TestStatus.PASSED: "\033[32m",
    TestStatus.FAILED: "\033[31m",
    TestStatus.SKIPPED: "\033[33m",
    "heading": "\033[36m",
    "reset": "\033[0m",
}
_SYMBOLS = {
    TestStatus.PASSED: "✓",
    TestStatus.FAILED: "✗",
    TestStatus.SKIPPED: "○",
}


@dataclass
class TestOutput:
    color_mode: str = "auto"
    stream: TextIO = field(default_factory=lambda: sys.stdout)

    def line(self, message: str = "") -> None:
        print(message, file=self.stream, flush=True)

    def heading(self, message: str) -> None:
        self.line(self._color(message, "heading"))

    def progress(self, name: str) -> None:
        marker = self._color("▶", "heading")
        print(f"{marker} {name} ... ", end="", file=self.stream, flush=True)

    def progress_result(self, result: TestResult) -> None:
        status = effective_status(result)
        self.line(self._color(_SYMBOLS[status], status))

    def command_output(self, stdout: str, stderr: str) -> None:
        for output in (stdout, stderr):
            if not output:
                continue
            print(
                output,
                end="" if output.endswith("\n") else "\n",
                file=self.stream,
                flush=True,
            )

    def result(self, name: str, result: TestResult, depth: int = 0) -> None:
        status = effective_status(result)
        symbol = self._color(_SYMBOLS[status], status)
        suffix = f" — {result.message}" if result.message else ""
        self.line(f"{'  ' * depth}{symbol} {name}{suffix}")
        if result.status == TestStatus.COMPOSITE and result.details:
            for child_name, child in result.details.items():
                self.result(child_name, child, depth + 1)

    def summary(self, name: str, result: TestResult) -> None:
        status = effective_status(result)
        symbol = self._color(_SYMBOLS[status], status)
        self.line(f"{symbol} {name}")

    def _color(self, text: str, key: TestStatus | str) -> str:
        if not self._should_color():
            return text
        return f"{_COLORS[key]}{text}{_COLORS['reset']}"

    def _should_color(self) -> bool:
        if self.color_mode == "never":
            return False
        if self.color_mode == "always":
            return True
        return self.stream.isatty()


def effective_status(result: TestResult) -> TestStatus:
    if result.status != TestStatus.COMPOSITE:
        return result.status
    if not result.details:
        return TestStatus.PASSED
    child_statuses = [effective_status(child) for child in result.details.values()]
    if TestStatus.FAILED in child_statuses:
        return TestStatus.FAILED
    if child_statuses and all(status == TestStatus.SKIPPED for status in child_statuses):
        return TestStatus.SKIPPED
    return TestStatus.PASSED


def has_failure(result: TestResult) -> bool:
    return effective_status(result) == TestStatus.FAILED
