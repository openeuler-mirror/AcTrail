from __future__ import annotations

import sys
import threading
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
    _lock: threading.RLock = field(
        default_factory=threading.RLock,
        init=False,
        repr=False,
    )
    _progress_name: str | None = field(default=None, init=False, repr=False)
    _progress_message: str | None = field(default=None, init=False, repr=False)

    def line(self, message: str = "") -> None:
        with self._lock:
            redraw = self._clear_progress_line()
            print(message, file=self.stream, flush=True)
            if redraw:
                self._render_progress_line()

    def heading(self, message: str) -> None:
        self.line(self._color(message, "heading"))

    def progress(self, name: str) -> None:
        with self._lock:
            if self._progress_name is not None:
                raise RuntimeError(
                    f"progress already active for {self._progress_name}"
                )
            self._progress_name = name
            self._progress_message = None
            if self._uses_in_place_progress():
                self._render_progress_line()
            else:
                print(
                    self._progress_text(),
                    file=self.stream,
                    flush=True,
                )

    def progress_update(self, message: str) -> None:
        with self._lock:
            if self._progress_name is None:
                raise RuntimeError("cannot update inactive progress")
            if message == self._progress_message:
                return
            self._progress_message = message
            if self._uses_in_place_progress():
                self._render_progress_line()
            else:
                print(
                    f"→ {self._progress_text()}",
                    file=self.stream,
                    flush=True,
                )

    def progress_result(self, result: TestResult) -> None:
        with self._lock:
            if self._progress_name is None:
                raise RuntimeError("cannot finish inactive progress")
            status = effective_status(result)
            symbol = self._color(_SYMBOLS[status], status)
            tail = symbol
            if status is TestStatus.SKIPPED and result.message:
                tail = f"{symbol} {result.message}"
            message = self._progress_text(tail)
            if self._uses_in_place_progress():
                print(
                    f"\r{message}\033[K",
                    file=self.stream,
                    flush=True,
                )
            else:
                print(f"→ {message}", file=self.stream, flush=True)
            self._progress_name = None
            self._progress_message = None

    def command_output(self, stdout: str, stderr: str) -> None:
        with self._lock:
            redraw = self._clear_progress_line()
            for output in (stdout, stderr):
                if not output:
                    continue
                print(
                    output,
                    end="" if output.endswith("\n") else "\n",
                    file=self.stream,
                    flush=True,
                )
            if redraw:
                self._render_progress_line()

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

    def _uses_in_place_progress(self) -> bool:
        return self.stream.isatty()

    def _progress_text(self, suffix: str | None = None) -> str:
        if self._progress_name is None:
            raise RuntimeError("progress is inactive")
        marker = self._color("▶", "heading")
        tail = suffix if suffix is not None else self._progress_message
        return f"{marker} {self._progress_name}...{tail or ''}"

    def _clear_progress_line(self) -> bool:
        redraw = (
            self._progress_name is not None
            and self._uses_in_place_progress()
        )
        if redraw:
            print("\r\033[K", end="", file=self.stream, flush=True)
        return redraw

    def _render_progress_line(self) -> None:
        print(
            f"\r{self._progress_text()}\033[K",
            end="",
            file=self.stream,
            flush=True,
        )


class CaseProgressReporter:
    """Routes semantic case progress to its log and selected console mode."""

    def __init__(
        self,
        console: TestOutput,
        log: TestOutput,
        *,
        detailed: bool,
    ):
        self._console = console
        self._log = log
        self._detailed = detailed
        self._lock = threading.RLock()
        self._last_progress: str | None = None

    def report(self, step: str, message: str | None = None) -> None:
        progress = self._format_progress(step, message)
        compact_progress = self._format_progress(step, None)
        with self._lock:
            if progress == self._last_progress:
                return
            self._last_progress = progress
            self._log.line(f"→ {progress}")
            if self._detailed:
                self._console.line(f"→ {progress}")
            else:
                self._console.progress_update(compact_progress)

    @staticmethod
    def _format_progress(step: str, message: str | None) -> str:
        normalized_step = " ".join(step.split())
        if not normalized_step:
            raise ValueError("progress step must not be empty")
        if message is None:
            return normalized_step
        normalized_message = " ".join(message.split())
        if not normalized_message:
            return normalized_step
        return f"{normalized_step}: {normalized_message}"


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
