"""Semantic action viewer checks."""

from __future__ import annotations

import json
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
        ("llm.call", "llm.request", "llm.response"),
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
            [
                str(actrailviewer),
                "--output-format",
                "json",
                "actions",
                "--config",
                str(config),
                "--trace-id",
                str(trace_id),
            ],
            echo=False,
        )
        document = parse_actions(output)
        found_kinds = {action.get("kind") for action in document.get("actions", [])}
        if all(kind in found_kinds for kind in required_kinds):
            print(
                f"viewer_actions_json_bytes={len(output.encode('utf-8'))}",
                flush=True,
            )
            return output
        time.sleep(sleep_sec)
    expected = ", ".join(required_kinds)
    raise RuntimeError(f"viewer actions did not show required action kinds: {expected}")


def require_complete_llm_action(actions: str) -> None:
    require_complete_action(actions, "llm.request")


def require_complete_llm_exchange(actions: str) -> None:
    require_complete_action(actions, "llm.call")
    require_complete_action(actions, "llm.request")
    require_complete_action(actions, "llm.response")


def require_llm_exchange_graph(actions: str) -> None:
    document = parse_actions(actions)
    by_id = {action["action_id"]: action for action in document.get("actions", [])}
    call_ids = complete_action_ids(document, "llm.call")
    request_ids = complete_action_ids(document, "llm.request")
    response_ids = complete_action_ids(document, "llm.response")
    if not call_ids:
        raise RuntimeError("actions did not contain a complete successful llm.call")
    if not request_ids:
        raise RuntimeError("actions did not contain a complete successful llm.request")
    if not response_ids:
        raise RuntimeError("actions did not contain a complete successful llm.response")
    links = document.get("links", [])
    if not any(
        link.get("role") == "llm.call.request"
        and link.get("parent_action_id") in call_ids
        and link.get("child_action_id") in request_ids
        for link in links
    ):
        raise RuntimeError("actions did not link llm.call to llm.request")
    if not any(
        link.get("role") == "llm.call.response"
        and link.get("parent_action_id") in call_ids
        and link.get("child_action_id") in response_ids
        for link in links
    ):
        raise RuntimeError("actions did not link llm.call to llm.response")
    if not any(
        link.get("role") == "llm.request.http_message"
        and link.get("parent_action_id") in request_ids
        and by_id.get(link.get("child_action_id"), {}).get("kind") == "http.message"
        for link in links
    ):
        raise RuntimeError("actions did not link llm.request to an http.message")
    if not any(
        link.get("role") in {"llm.response.http_message", "llm.response.sse_stream"}
        and link.get("parent_action_id") in response_ids
        for link in links
    ):
        raise RuntimeError("actions did not link llm.response to response facts")


def require_complete_action(actions: str, kind: str) -> None:
    if complete_action_ids(parse_actions(actions), kind):
        return
    raise RuntimeError(f"actions did not contain a complete successful {kind}")


def count_action_rows(actions: str) -> int:
    return len(parse_actions(actions).get("actions", []))


def parse_actions(actions: str) -> dict:
    try:
        document = json.loads(actions)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"viewer actions output was not JSON: {error}") from error
    if not isinstance(document, dict) or not isinstance(document.get("actions"), list):
        raise RuntimeError("viewer actions JSON must contain an actions list")
    if not isinstance(document.get("links", []), list):
        raise RuntimeError("viewer actions JSON links must be a list")
    return document


def complete_action_ids(document: dict, kind: str) -> set[str]:
    return {
        action["action_id"]
        for action in document.get("actions", [])
        if action.get("kind") == kind
        and action.get("status") == "success"
        and action.get("completeness") == "complete"
        and isinstance(action.get("action_id"), str)
    }
