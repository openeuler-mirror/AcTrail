"""Run one regression E2E step and print its check immediately."""

from __future__ import annotations

import contextlib
import io
from typing import Callable, TypeVar

from model import FAIL, PASS, CaseResult


T = TypeVar("T")


class StepFailure(RuntimeError):
    """Stops a case after its current step has already been recorded."""


def run_step(
    result: CaseResult,
    name: str,
    action: Callable[[], T],
    detail: Callable[[T], str] | str,
    evidence: Callable[[T], str] | str,
    *,
    failure_status: str = FAIL,
    failure_evidence: str = "the step failed before its expected state was reached",
    progress: bool = False,
) -> T:
    if progress:
        result.begin_check(name)
    try:
        value, _ = capture_stdout(action)
    except Exception as error:
        result.status = failure_status
        result.add_check(name, failure_status, str(error), failure_evidence)
        raise StepFailure(str(error)) from error
    result.add_check(name, PASS, resolve_text(detail, value), resolve_text(evidence, value))
    return value


def capture_stdout(action: Callable[[], T]) -> tuple[T, str]:
    stream = io.StringIO()
    with contextlib.redirect_stdout(stream), contextlib.redirect_stderr(stream):
        value = action()
    return value, stream.getvalue()


def resolve_text(value: Callable[[T], str] | str, step_value: T) -> str:
    if callable(value):
        return value(step_value)
    return value
