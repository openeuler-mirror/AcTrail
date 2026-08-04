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
    process = subprocess.Popen(
        actrail_command(
            actrailweb,
            config,
            "--addr",
            host,
            "--port",
            str(port),
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


def require_web_time_attribution(
    actrailweb: Path,
    config: Path | None,
    trace_id: int,
    timeout_seconds: float,
    poll_interval_seconds: float,
    *,
    require_tool: bool = False,
) -> dict[str, object]:
    """Validate time attribution through the real Web API and captured Agent data."""
    values = read_config(operator_config_path(config))
    host = web_host(required(values, "web_listen_addr"))
    port = reserve_local_port(host)
    process = subprocess.Popen(
        actrail_command(
            actrailweb,
            config,
            "--addr",
            host,
            "--port",
            str(port),
        ),
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    origin = f"http://{host}:{port}"
    action_tree_url = f"{origin}/api/traces/{trace_id}/action-tree"
    attribution_url = f"{origin}/api/traces/{trace_id}/time-attribution"
    try:
        wait_for_action_tree_root(
            action_tree_url,
            timeout_seconds,
            poll_interval_seconds,
        )
        action_tree = fetch_action_tree_json(
            action_tree_url,
            "",
            timeout_seconds,
        )
        attribution = wait_for_time_attribution(
            attribution_url,
            timeout_seconds,
            poll_interval_seconds,
        )
        summary = validate_time_attribution(
            attribution,
            action_tree,
            require_tool=require_tool,
        )
        validate_aggregate_time_attribution(
            origin,
            attribution,
            timeout_seconds,
        )
        print(
            "web_time_attribution "
            f"trace={trace_id} "
            f"total_nanos={summary['total_nanos']} "
            f"agent_nanos={summary['agent_nanos']} "
            f"model_nanos={summary['model_nanos']} "
            f"unattributed_nanos={summary['unattributed_nanos']} "
            f"segments={summary['segment_count']} "
            f"tools={summary['named_tool_count']} "
            f"commands={summary['actual_command_count']}",
            flush=True,
        )
        return summary
    except Exception as error:
        output = collect_process_output(process)
        raise RuntimeError(f"{error}\nactrailweb_output={output}") from error
    finally:
        stop_web_process(process, timeout_seconds)


def wait_for_time_attribution(
    url: str,
    timeout_seconds: float,
    poll_interval_seconds: float,
) -> dict:
    deadline = time.monotonic() + timeout_seconds
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            attribution = fetch_json_url(url, timeout_seconds)
            if attribution.get("scope", {}).get("duration_nanos") is not None:
                return attribution
        except Exception as error:
            last_error = error
        sleep_seconds = min(
            poll_interval_seconds,
            max(deadline - time.monotonic(), 0),
        )
        if sleep_seconds > 0:
            time.sleep(sleep_seconds)
    raise RuntimeError(f"actrailweb time attribution was not ready: {last_error}")


def validate_time_attribution(
    attribution: dict,
    action_tree: dict,
    *,
    require_tool: bool,
) -> dict[str, object]:
    if attribution.get("schema_version") != "time-attribution.v1":
        raise RuntimeError("time attribution schema version is missing or unexpected")
    if attribution.get("status") != "complete":
        raise RuntimeError(
            "terminal real-Agent Trace time attribution was not complete: "
            f"{attribution.get('status')} issues={attribution.get('issues')}"
        )
    scope = attribution.get("scope", {})
    scope_start = required_decimal(scope, "start_unix_nanos")
    scope_end = required_decimal(scope, "end_unix_nanos")
    total = required_decimal(scope, "duration_nanos")
    if scope_end - scope_start != total or total <= 0:
        raise RuntimeError("time attribution scope boundaries do not match its duration")

    categories = keyed_rows(attribution.get("categories"), "time attribution categories")
    expected_keys = {"agent_side", "model_side", "unattributed"}
    if set(categories) != expected_keys:
        raise RuntimeError(
            f"time attribution categories mismatch: expected {expected_keys}, found {set(categories)}"
        )
    category_total = sum(required_decimal(row, "duration_nanos") for row in categories.values())
    if category_total != total:
        raise RuntimeError(
            f"time attribution categories sum to {category_total}, expected {total}"
        )
    percentage_total = sum(int(row.get("percentage_bps", -1)) for row in categories.values())
    if percentage_total != 10_000:
        raise RuntimeError(
            f"time attribution percentages sum to {percentage_total}, expected 10000 bps"
        )
    if required_decimal(categories["model_side"], "duration_nanos") <= 0:
        raise RuntimeError("real-Agent Trace has no observable model-side time")

    segments = attribution.get("segments")
    if not isinstance(segments, list) or not segments:
        raise RuntimeError("time attribution did not return exclusive segments")
    segment_category_totals = {key: 0 for key in expected_keys}
    cursor = scope_start
    model_intervals: list[tuple[int, int]] = []
    for segment in segments:
        start = required_decimal(segment, "start_unix_nanos")
        end = required_decimal(segment, "end_unix_nanos")
        duration = required_decimal(segment, "duration_nanos")
        category = segment.get("category")
        if start != cursor:
            raise RuntimeError(
                f"time attribution segments are not contiguous at {cursor}: next={start}"
            )
        if end <= start or end - start != duration:
            raise RuntimeError(f"invalid time attribution segment boundaries: {segment}")
        if category not in segment_category_totals:
            raise RuntimeError(f"unknown time attribution segment category: {category}")
        segment_category_totals[category] += duration
        if category == "model_side":
            model_intervals.append((start, end))
        cursor = end
    if cursor != scope_end:
        raise RuntimeError(
            f"time attribution segments end at {cursor}, expected scope end {scope_end}"
        )
    for key, duration in segment_category_totals.items():
        expected = required_decimal(categories[key], "duration_nanos")
        if duration != expected:
            raise RuntimeError(
                f"segment total for {key} is {duration}, category reports {expected}"
            )

    model_breakdown_total = sum(
        required_decimal(row, "duration_nanos")
        for row in attribution.get("models", [])
    )
    if model_breakdown_total != segment_category_totals["model_side"]:
        raise RuntimeError(
            "model breakdown does not partition observable model-side time"
        )
    agent_breakdown_total = sum(
        required_decimal(row, "duration_nanos")
        for row in attribution.get("tools", [])
    )
    if agent_breakdown_total != segment_category_totals["agent_side"]:
        raise RuntimeError("tool/local breakdown does not partition Agent-side time")

    tool_scoped_total = sum(
        required_decimal(segment, "duration_nanos")
        for segment in segments
        if segment.get("category") == "agent_side"
        and segment.get("key") != "__orchestration__"
    )
    command_segments = attribution.get("command_segments")
    if not isinstance(command_segments, list):
        raise RuntimeError("time attribution command segments are missing")
    command_segment_total = 0
    previous_command_end = scope_start
    for segment in command_segments:
        start = required_decimal(segment, "start_unix_nanos")
        end = required_decimal(segment, "end_unix_nanos")
        duration = required_decimal(segment, "duration_nanos")
        if start < previous_command_end or end <= start or end - start != duration:
            raise RuntimeError(
                f"command attribution overlaps or has invalid boundaries: {segment}"
            )
        if not segment.get("agent_tools"):
            raise RuntimeError(
                f"command attribution is missing its logical Agent Tool: {segment}"
            )
        command_segment_total += duration
        previous_command_end = end
    if command_segment_total != tool_scoped_total:
        raise RuntimeError(
            "command segments do not exclusively partition Agent Tool time: "
            f"commands={command_segment_total} tools={tool_scoped_total}"
        )
    command_breakdown_total = sum(
        required_decimal(row, "duration_nanos")
        for row in attribution.get("commands", [])
    )
    if command_breakdown_total != command_segment_total:
        raise RuntimeError(
            "command breakdown does not match exclusive command segments"
        )
    validate_dominant_attribution_targets(
        attribution.get("models", []),
        [
            segment
            for segment in segments
            if segment.get("category") == "model_side"
        ],
        "model",
    )
    validate_dominant_attribution_targets(
        attribution.get("tools", []),
        [
            segment
            for segment in segments
            if segment.get("category") == "agent_side"
        ],
        "Agent Tool",
    )
    validate_dominant_attribution_targets(
        attribution.get("commands", []),
        command_segments,
        "command",
    )
    bottleneck_counts = validate_time_attribution_bottlenecks(
        attribution,
        action_tree,
        segments,
        command_segments,
        scope_start,
        scope_end,
    )

    validate_round_partition(attribution.get("rounds"), scope_start, scope_end)
    validate_llm_calls_covered(action_tree, model_intervals, scope_start, scope_end)

    named_tools = [
        row
        for row in attribution.get("tools", [])
        if row.get("key")
        not in {
            "__orchestration__",
            "__unidentified_command__",
            "__concurrent_tools__",
        }
        and required_decimal(row, "duration_nanos") > 0
        and int(row.get("action_count", 0)) > 0
    ]
    if require_tool and not named_tools:
        raise RuntimeError(
            "real-Agent tool task did not expose a named Agent-side tool interval"
        )
    if require_tool and not command_segments:
        raise RuntimeError(
            "real-Agent tool task did not expose its command/tool-overhead partition"
        )
    actual_commands = [
        row
        for row in attribution.get("commands", [])
        if row.get("kind") == "command"
        and required_decimal(row, "duration_nanos") > 0
    ]
    action_ids = {
        action.get("id")
        for action in action_tree.get("actions", [])
        if action.get("id")
    }
    for command in actual_commands:
        target_ids = set(command.get("target", {}).get("action_ids", []))
        if not target_ids.intersection(action_ids):
            raise RuntimeError(
                "actual command attribution target cannot be located in Waterfall: "
                f"{command.get('key')}"
            )

    return {
        "total_nanos": total,
        "agent_nanos": segment_category_totals["agent_side"],
        "model_nanos": segment_category_totals["model_side"],
        "unattributed_nanos": segment_category_totals["unattributed"],
        "segment_count": len(segments),
        "named_tool_count": len(named_tools),
        "actual_command_count": len(actual_commands),
        "model_bottleneck_count": bottleneck_counts["model_requests"],
        "command_bottleneck_count": bottleneck_counts["commands"],
        "unattributed_bottleneck_count": bottleneck_counts["unattributed_gaps"],
    }


def validate_dominant_attribution_targets(
    breakdown: object,
    segments: list[dict],
    label: str,
) -> None:
    if not isinstance(breakdown, list):
        raise RuntimeError(f"{label} attribution breakdown is missing")
    durations_by_key: dict[str, list[int]] = {}
    for segment in segments:
        key = segment.get("key")
        if isinstance(key, str):
            durations_by_key.setdefault(key, []).append(
                required_decimal(segment, "duration_nanos")
            )
    for row in breakdown:
        key = row.get("key")
        durations = durations_by_key.get(key, [])
        target = row.get("target")
        if not durations or not isinstance(target, dict):
            raise RuntimeError(f"{label} attribution target is missing for {key}")
        target_duration = (
            required_decimal(target, "end_unix_nanos")
            - required_decimal(target, "start_unix_nanos")
        )
        if target_duration != max(durations):
            raise RuntimeError(
                f"{label} attribution target for {key} is not its longest interval: "
                f"target={target_duration} longest={max(durations)}"
            )


def validate_time_attribution_bottlenecks(
    attribution: dict,
    action_tree: dict,
    segments: list[dict],
    command_segments: list[dict],
    scope_start: int,
    scope_end: int,
) -> dict[str, int]:
    bottlenecks = attribution.get("bottlenecks")
    if not isinstance(bottlenecks, dict):
        raise RuntimeError("time attribution bottleneck ranking is missing")
    default_display_limit = int(bottlenecks.get("default_display_limit", 0))
    if default_display_limit <= 0:
        raise RuntimeError("time attribution bottleneck display limit is invalid")

    known_action_ids = {
        action.get("id")
        for action in action_tree.get("actions", [])
        if isinstance(action.get("id"), str)
    }
    model_spans = action_spans(
        segment
        for segment in segments
        if segment.get("category") == "model_side"
    )
    command_spans = action_spans(
        segment
        for segment in command_segments
        if segment.get("kind") in {"command", "concurrent_commands"}
    )
    unattributed_spans = [
        (
            required_decimal(segment, "start_unix_nanos"),
            required_decimal(segment, "end_unix_nanos"),
            (),
        )
        for segment in segments
        if segment.get("category") == "unattributed"
    ]
    sources = {
        "model_requests": [
            (start, end, (action_id,))
            for action_id, (start, end) in model_spans.items()
        ],
        "commands": [
            (start, end, (action_id,))
            for action_id, (start, end) in command_spans.items()
        ],
        "unattributed_gaps": unattributed_spans,
    }
    kinds = {
        "model_requests": "model_request",
        "commands": "command_occurrence",
        "unattributed_gaps": "unattributed_gap",
    }
    counts: dict[str, int] = {}
    for collection_name, expected_spans in sources.items():
        collection = bottlenecks.get(collection_name)
        if not isinstance(collection, dict):
            raise RuntimeError(f"{collection_name} bottleneck collection is missing")
        observed_count = int(collection.get("observed_count", -1))
        if observed_count != len(expected_spans):
            raise RuntimeError(
                f"{collection_name} bottleneck count is {observed_count}, "
                f"expected {len(expected_spans)}"
            )
        items = collection.get("items")
        if not isinstance(items, list):
            raise RuntimeError(f"{collection_name} bottleneck items are missing")
        if len(items) != observed_count:
            raise RuntimeError(
                f"{collection_name} returned {len(items)} occurrences, "
                f"expected all {observed_count} observed occurrences"
            )
        expected = sorted(
            expected_spans,
            key=lambda span: (-(span[1] - span[0]), span[0], span[2]),
        )
        actual: list[tuple[int, int, tuple[str, ...]]] = []
        for item in items:
            start = required_decimal(item, "start_unix_nanos")
            end = required_decimal(item, "end_unix_nanos")
            duration = required_decimal(item, "duration_nanos")
            action_ids = tuple(item.get("action_ids", []))
            if item.get("kind") != kinds[collection_name]:
                raise RuntimeError(
                    f"{collection_name} bottleneck kind is unexpected: {item}"
                )
            if start < scope_start or end > scope_end or end <= start or end - start != duration:
                raise RuntimeError(
                    f"{collection_name} bottleneck boundaries are invalid: {item}"
                )
            if collection_name != "unattributed_gaps" and (
                len(action_ids) != 1 or action_ids[0] not in known_action_ids
            ):
                raise RuntimeError(
                    f"{collection_name} bottleneck cannot be located in Waterfall: {item}"
                )
            if collection_name == "unattributed_gaps" and action_ids:
                raise RuntimeError(
                    f"unattributed bottleneck unexpectedly links an action: {item}"
                )
            actual.append((start, end, action_ids))
        if actual != expected:
            raise RuntimeError(
                f"{collection_name} bottlenecks are not the longest source intervals: "
                f"actual={actual} expected={expected}"
            )
        counts[collection_name] = len(items)
    return counts


def action_spans(segments: object) -> dict[str, tuple[int, int]]:
    spans: dict[str, tuple[int, int]] = {}
    for segment in segments:
        start = required_decimal(segment, "start_unix_nanos")
        end = required_decimal(segment, "end_unix_nanos")
        for action_id in segment.get("action_ids", []):
            if not isinstance(action_id, str):
                continue
            previous = spans.get(action_id)
            spans[action_id] = (
                min(previous[0], start) if previous else start,
                max(previous[1], end) if previous else end,
            )
    return spans


def validate_round_partition(rounds: object, scope_start: int, scope_end: int) -> None:
    if not isinstance(rounds, list) or not rounds:
        raise RuntimeError("time attribution did not return round attribution")
    cursor = scope_start
    for round_row in rounds:
        start = required_decimal(round_row, "start_unix_nanos")
        end = required_decimal(round_row, "end_unix_nanos")
        duration = required_decimal(round_row, "duration_nanos")
        if start != cursor or end <= start or end - start != duration:
            raise RuntimeError(f"round attribution is not a valid partition: {round_row}")
        category_total = sum(
            required_decimal(row, "duration_nanos")
            for row in round_row.get("categories", [])
        )
        if category_total != duration:
            raise RuntimeError(
                f"round categories sum to {category_total}, expected {duration}"
            )
        percentage_total = sum(
            int(row.get("percentage_bps", -1))
            for row in round_row.get("categories", [])
        )
        if percentage_total != 10_000:
            raise RuntimeError(
                f"round percentages sum to {percentage_total}, expected 10000 bps"
            )
        cursor = end
    if cursor != scope_end:
        raise RuntimeError(
            f"round attribution ends at {cursor}, expected scope end {scope_end}"
        )


def validate_llm_calls_covered(
    action_tree: dict,
    model_intervals: list[tuple[int, int]],
    scope_start: int,
    scope_end: int,
) -> None:
    calls = [
        action
        for action in action_tree.get("actions", [])
        if action.get("kind") == "llm.call"
        and action.get("end_time_unix_nanos") is not None
    ]
    if not calls:
        raise RuntimeError("real-Agent action tree has no completed llm.call")
    for call in calls:
        start = max(required_decimal(call, "start_time_unix_nanos"), scope_start)
        end = min(required_decimal(call, "end_time_unix_nanos"), scope_end)
        if start >= end:
            continue
        cursor = start
        for model_start, model_end in model_intervals:
            if model_end <= cursor:
                continue
            if model_start > cursor:
                break
            cursor = max(cursor, model_end)
            if cursor >= end:
                break
        if cursor < end:
            raise RuntimeError(
                "observable model-side segments do not cover complete llm.call "
                f"{call.get('id')}: uncovered [{cursor}, {end})"
            )


def validate_aggregate_time_attribution(
    origin: str,
    trace_attribution: dict,
    timeout_seconds: float,
) -> None:
    scope = trace_attribution["scope"]
    start_nanos = required_decimal(scope, "start_unix_nanos")
    end_nanos = required_decimal(scope, "end_unix_nanos")
    from_ms = start_nanos // 1_000_000
    to_ms = (end_nanos + 999_999) // 1_000_000
    query = urllib.parse.urlencode({"from_ms": from_ms, "to_ms": to_ms})
    aggregate = fetch_json_url(
        f"{origin}/api/stats/time-attribution/activity?{query}",
        timeout_seconds,
    )
    total = required_decimal(aggregate, "total_duration_nanos")
    categories = aggregate.get("categories", [])
    if sum(required_decimal(row, "duration_nanos") for row in categories) != total:
        raise RuntimeError("aggregate time attribution categories do not sum to total")
    if total > 0 and sum(int(row.get("percentage_bps", -1)) for row in categories) != 10_000:
        raise RuntimeError("aggregate time attribution percentages do not sum to 10000 bps")
    if int(aggregate.get("coverage", {}).get("trace_count", 0)) < 1:
        raise RuntimeError("aggregate time attribution did not include the real-Agent Trace")
    aggregate_agent = next(
        (
            required_decimal(row, "duration_nanos")
            for row in categories
            if row.get("key") == "agent_side"
        ),
        0,
    )
    aggregate_commands = sum(
        required_decimal(row, "duration_nanos")
        for row in aggregate.get("commands", [])
    )
    if aggregate_commands > aggregate_agent:
        raise RuntimeError("aggregate command time exceeds aggregate Agent-side time")

    rows_query = urllib.parse.urlencode(
        {
            "from_ms": from_ms,
            "to_ms": to_ms,
            "offset": 0,
            "limit": 50,
            "dimension": "category",
            "key": "model_side",
        }
    )
    rows = fetch_json_url(
        f"{origin}/api/stats/time-attribution/rows?{rows_query}",
        timeout_seconds,
    )
    trace_id = trace_attribution.get("trace", {}).get("id")
    matching = [
        row
        for row in rows.get("rows", [])
        if row.get("trace", {}).get("id") == trace_id
    ]
    if not matching or not matching[0].get("target"):
        raise RuntimeError(
            "aggregate model-side drill-down did not return the real Trace interval"
        )

    command_rows = trace_attribution.get("commands", [])
    if command_rows:
        command = next(
            (row for row in command_rows if row.get("kind") == "command"),
            command_rows[0],
        )
        command_query = urllib.parse.urlencode(
            {
                "from_ms": from_ms,
                "to_ms": to_ms,
                "offset": 0,
                "limit": 50,
                "dimension": "command",
                "key": command.get("key"),
            }
        )
        command_drilldown = fetch_json_url(
            f"{origin}/api/stats/time-attribution/rows?{command_query}",
            timeout_seconds,
        )
        command_matching = [
            row
            for row in command_drilldown.get("rows", [])
            if row.get("trace", {}).get("id") == trace_id
        ]
        if not command_matching or not command_matching[0].get("target"):
            raise RuntimeError(
                "aggregate command drill-down did not return the real Trace interval"
            )


def keyed_rows(rows: object, label: str) -> dict[str, dict]:
    if not isinstance(rows, list):
        raise RuntimeError(f"{label} are missing")
    output = {}
    for row in rows:
        key = row.get("key")
        if not isinstance(key, str):
            raise RuntimeError(f"{label} contain a row without a string key")
        output[key] = row
    return output


def required_decimal(row: dict, key: str) -> int:
    raw = row.get(key)
    if not isinstance(raw, str) or not raw.isdecimal():
        raise RuntimeError(f"{key} is not a decimal string: {raw!r}")
    return int(raw)


def fetch_json_url(url: str, timeout_seconds: float) -> dict:
    with urllib.request.urlopen(url, timeout=timeout_seconds) as response:
        return json.loads(response.read().decode("utf-8"))


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
    page_limit = action_tree_page_limit(full)
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
            action_tree_children_path(parent_id, page_limit),
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


def action_tree_page_limit(full_tree: dict) -> int:
    action_count = len(
        [
            action
            for action in full_tree.get("actions", [])
            if isinstance(action.get("id"), str)
        ]
    )
    if action_count == 0:
        raise RuntimeError("web action-tree has no display actions")
    return action_count


def action_tree_children_path(parent_id: str, page_limit: int) -> str:
    encoded_parent = urllib.parse.quote(parent_id, safe="")
    return f"/children/{encoded_parent}?offset=0&limit={page_limit}"


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
