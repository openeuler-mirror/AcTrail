"""Semantic action viewer checks."""

from __future__ import annotations

import time
from pathlib import Path

from .config import run_checked


def wait_for_actions(
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    attempts: int,
    sleep_sec: float,
) -> str:
    return wait_for_actions_with_kinds(
        actrailviewer,
        config,
        trace_id,
        attempts,
        sleep_sec,
        ("llm.request",),
    )


def wait_for_llm_exchange_actions(
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    attempts: int,
    sleep_sec: float,
) -> str:
    return wait_for_actions_with_kinds(
        actrailviewer,
        config,
        trace_id,
        attempts,
        sleep_sec,
        ("llm.request", "llm.response"),
    )


def wait_for_actions_with_kinds(
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    attempts: int,
    sleep_sec: float,
    required_kinds: tuple[str, ...],
) -> str:
    for _ in range(attempts):
        output = run_checked(
            [str(actrailviewer), "actions", "--config", str(config), "--trace-id", str(trace_id)],
            echo=False,
        )
        if all(kind in output for kind in required_kinds):
            print(output, end="", flush=True)
            return output
        time.sleep(sleep_sec)
    expected = ", ".join(required_kinds)
    raise RuntimeError(f"viewer actions did not show required action kinds: {expected}")


def require_complete_llm_action(actions: str) -> None:
    require_complete_action(actions, "llm.request")


def require_complete_llm_exchange(actions: str) -> None:
    require_complete_action(actions, "llm.request")
    require_complete_action(actions, "llm.response")


def require_complete_action(actions: str, kind: str) -> None:
    for line in actions.splitlines():
        if kind in line and "success" in line and "complete" in line:
            return
    raise RuntimeError(f"actions did not contain a complete successful {kind}")
