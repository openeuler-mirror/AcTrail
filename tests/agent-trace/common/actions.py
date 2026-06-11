"""Semantic action viewer checks."""

from __future__ import annotations

import json
import socket
import subprocess
import time
import urllib.parse
import urllib.request
from pathlib import Path

from .config import actrail_command, operator_config_path, read_config, required, run_checked

EPHEMERAL_PORT = 0
NODE_ID_AGENT = "agent-process"


def wait_for_actions(
    actrailviewer: Path,
    config: Path | None,
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
    config: Path | None,
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
    config: Path | None,
    trace_id: int,
    attempts: int,
    sleep_sec: float,
    required_kinds: tuple[str, ...],
) -> str:
    for _ in range(attempts):
        output = run_checked(
            actrail_command(
                actrailviewer,
                config,
                "--output-format",
                "json",
                "actions",
                "--trace-id",
                str(trace_id),
            ),
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


def require_web_action_tree_projection(
    actrailweb: Path,
    config: Path | None,
    trace_id: int,
    timeout_seconds: float,
    poll_interval_seconds: float,
    *,
    required_reachable_kinds: tuple[str, ...] = (),
    required_root_linkless_kinds: tuple[str, ...] = (),
    forbidden_root_linkless_kinds: tuple[str, ...] = (),
    required_parent_child_kinds: tuple[tuple[str, str], ...] = (),
) -> dict[str, object]:
    values = read_config(operator_config_path(config))
    host = web_host(required(values, "web_listen_addr"))
    port = reserve_local_port(host)
    request_read_timeout_ms = required(values, "web_request_read_timeout_ms")
    process = subprocess.Popen(
        actrail_command(
            actrailweb,
            config,
            "--addr",
            host,
            "--port",
            str(port),
            "--request-read-timeout-ms",
            request_read_timeout_ms,
        ),
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    base_url = f"http://{host}:{port}/api/traces/{trace_id}/action-tree"
    try:
        summary = read_web_action_tree_projection(
            base_url,
            timeout_seconds,
            poll_interval_seconds,
            required_reachable_kinds,
            required_root_linkless_kinds,
            forbidden_root_linkless_kinds,
            required_parent_child_kinds,
        )
        print(
            "web_action_tree "
            f"actions={summary['action_count']} "
            f"reachable={summary['reachable_count']} "
            f"http_messages={summary['kind_counts'].get('http.message', 0)} "
            f"root_linkless={summary['root_linkless_count']}",
            flush=True,
        )
        return summary
    except Exception as error:
        output = collect_process_output(process)
        raise RuntimeError(f"{error}\nactrailweb_output={output}") from error
    finally:
        stop_web_process(process, timeout_seconds)


def read_web_action_tree_projection(
    base_url: str,
    timeout_seconds: float,
    poll_interval_seconds: float,
    required_reachable_kinds: tuple[str, ...],
    required_root_linkless_kinds: tuple[str, ...],
    forbidden_root_linkless_kinds: tuple[str, ...],
    required_parent_child_kinds: tuple[tuple[str, str], ...],
) -> dict[str, object]:
    wait_for_action_tree_root(base_url, timeout_seconds, poll_interval_seconds)
    full = fetch_action_tree_json(base_url, "", timeout_seconds)
    seen: set[str] = set()
    stack = [NODE_ID_AGENT]
    kind_counts: dict[str, int] = {}
    kind_by_id: dict[str, str] = {}
    parent_child_kind_counts: dict[tuple[str, str], int] = {}
    root_linkless_kinds: list[str] = []
    while stack:
        parent_id = stack.pop()
        parent_kind = kind_by_id.get(parent_id)
        children = fetch_action_tree_json(
            base_url,
            f"/children/{urllib.parse.quote(parent_id, safe='')}",
            timeout_seconds,
        )
        child_state = {row["id"]: row for row in children.get("child_state", [])}
        linked_child_ids = {link.get("child") for link in children.get("links", [])}
        for action in children.get("actions", []):
            action_id = action.get("id")
            if not isinstance(action_id, str):
                raise RuntimeError("action-tree child action is missing a string id")
            if parent_id == NODE_ID_AGENT and action_id not in linked_child_ids:
                root_linkless_kinds.append(action.get("kind", ""))
            if action_id in seen:
                continue
            seen.add(action_id)
            kind = action.get("kind", "")
            kind_by_id[action_id] = kind
            kind_counts[kind] = kind_counts.get(kind, 0) + 1
            if parent_kind is not None:
                pair = (parent_kind, kind)
                parent_child_kind_counts[pair] = parent_child_kind_counts.get(pair, 0) + 1
            state = child_state.get(action_id, {})
            if state.get("has_children") and state.get("child_count", 0) > 0:
                stack.append(action_id)
    all_ids = {
        action["id"]
        for action in full.get("actions", [])
        if isinstance(action.get("id"), str)
    }
    missing = sorted(all_ids - seen)
    if missing:
        raise RuntimeError("web action-tree has unreachable display actions: " + ", ".join(missing))
    missing_kinds = [kind for kind in required_reachable_kinds if kind_counts.get(kind, 0) == 0]
    if missing_kinds:
        raise RuntimeError(
            "web action-tree did not expose required reachable kinds: "
            + ", ".join(missing_kinds)
        )
    missing_root_linkless = [
        kind for kind in required_root_linkless_kinds if kind not in root_linkless_kinds
    ]
    if missing_root_linkless:
        raise RuntimeError(
            "web action-tree did not expose required root fallback kinds without links: "
            + ", ".join(missing_root_linkless)
        )
    forbidden_root_linkless = [
        kind for kind in forbidden_root_linkless_kinds if kind in root_linkless_kinds
    ]
    if forbidden_root_linkless:
        raise RuntimeError(
            "web action-tree exposed forbidden root fallback kinds without links: "
            + ", ".join(forbidden_root_linkless)
        )
    missing_parent_child = [
        (parent, child)
        for parent, child in required_parent_child_kinds
        if parent_child_kind_counts.get((parent, child), 0) == 0
    ]
    if missing_parent_child:
        raise RuntimeError(
            "web action-tree did not expose required parent child kind pairs: "
            + ", ".join(f"{parent}->{child}" for parent, child in missing_parent_child)
        )
    return {
        "action_count": len(all_ids),
        "reachable_count": len(seen),
        "kind_counts": kind_counts,
        "parent_child_kind_counts": parent_child_kind_counts,
        "root_linkless_count": len(root_linkless_kinds),
        "root_linkless_kinds": root_linkless_kinds,
    }


def wait_for_action_tree_root(
    base_url: str,
    timeout_seconds: float,
    poll_interval_seconds: float,
) -> None:
    deadline = time.monotonic() + timeout_seconds
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            fetch_action_tree_json(base_url, "/root", timeout_seconds)
            return
        except Exception as error:
            last_error = error
            sleep_seconds = min(poll_interval_seconds, max(deadline - time.monotonic(), 0))
            if sleep_seconds > 0:
                time.sleep(sleep_seconds)
    raise RuntimeError(f"actrailweb action-tree root was not ready: {last_error}")


def fetch_action_tree_json(base_url: str, path: str, timeout_seconds: float) -> dict:
    with urllib.request.urlopen(base_url + path, timeout=timeout_seconds) as response:
        return json.loads(response.read().decode("utf-8"))


def web_host(listen_addr: str) -> str:
    host, separator, _port = listen_addr.rpartition(":")
    if not separator or not host:
        raise RuntimeError(f"invalid web_listen_addr: {listen_addr}")
    return host.strip("[]")


def reserve_local_port(host: str) -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as server:
        server.bind((host, EPHEMERAL_PORT))
        return int(server.getsockname()[1])


def stop_web_process(process: subprocess.Popen[str], timeout_seconds: float) -> None:
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=timeout_seconds)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=timeout_seconds)


def collect_process_output(process: subprocess.Popen[str]) -> str:
    if process.stdout is None:
        return ""
    try:
        output, _ = process.communicate(timeout=0)
        return output
    except subprocess.TimeoutExpired:
        return ""


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
