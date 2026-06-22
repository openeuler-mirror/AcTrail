#!/usr/bin/env python3
"""Verify HTTP socket payloads are projected and exported as llm.request."""

from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "agent-trace"))
from common import (  # noqa: E402
    actrail_command,
    clean_configured_paths,
    emit_llm_otel_evidence,
    export_otel,
    launch_and_parse_trace,
    otel_attrs,
    otel_spans,
    read_config,
    repo_root,
    require_binary,
    require_complete_llm_action,
    require_web_action_tree_projection,
    require_otel_span,
    require_root,
    required,
    run_checked,
    start_daemon,
    stop_process,
    wait_for_actions,
    wait_for_payloads,
)


def main() -> int:
    args = parse_args()
    require_root()
    repo = repo_root()
    case_dir = Path(__file__).resolve().parent
    config = resolve_path(args.config, repo)
    settings = read_config(resolve_path(args.workload_config, repo))
    bin_dir = resolve_path(args.bin_dir, repo)
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
    actrailweb = require_binary(bin_dir, "actrailweb")
    workload = case_dir / "workload.py"

    clean_configured_paths(actrailctl, config)
    daemon = start_daemon(
        actraild,
        config,
        float(required(settings, "daemon_ready_timeout_seconds")),
    )
    try:
        trace_id, output = launch_and_parse_trace(
            actrailctl,
            config,
            "http-llm-projection",
            workload_argv(workload, settings),
            float(required(settings, "launch_timeout_seconds")),
        )
        if "llm projection workload complete" not in output:
            raise RuntimeError("workload did not report completion")
        workload_pid = parse_workload_pid(output)
        summary = trace_summary(actrailviewer, config, trace_id)
        require_launch_root_pid(summary, workload_pid)
        payloads = wait_for_payloads(
            actrailctl,
            actrailviewer,
            config,
            trace_id,
            int(required(settings, "drain_attempts")),
            float(required(settings, "drain_sleep_seconds")),
            required(settings, "payload_head"),
            ["Syscall", "socket-syscall", "Complete", "success"],
        )
        payload_count = require_complete_outbound_socket_payload_rows(payloads)
        actions = wait_for_actions(
            actrailviewer,
            config,
            trace_id,
            int(required(settings, "drain_attempts")),
            float(required(settings, "drain_sleep_seconds")),
        )
        require_complete_llm_action(actions)
        web_tree = require_web_action_tree_projection(
            actrailweb,
            config,
            trace_id,
            float(required(settings, "daemon_ready_timeout_seconds")),
            float(required(settings, "drain_sleep_seconds")),
            required_reachable_kinds=("llm.request", "http.message"),
            forbidden_root_linkless_kinds=("http.message",),
            required_parent_child_kinds=(("command.invocation", "http.message"),),
        )
        otel = export_otel(
            actrailviewer,
            config,
            trace_id,
            Path(required(settings, "otel_output_path")),
        )
        span_count = require_otel_span(otel, "llm.request")
        marker = required(settings, "marker")
        require_http_llm_span(otel, marker, required(settings, "model"))
        emit_llm_otel_evidence(otel, int(required(settings, "evidence_text_max_chars")))
        print(f"http_llm_projection_trace_id={trace_id}")
        print(f"http_llm_projection_root_pid={workload_pid}")
        print(f"http_llm_projection_payload_segments={payload_count}")
        print(f"http_llm_projection_web_action_tree_reachable={web_tree['reachable_count']}")
        print(f"http_llm_projection_spans={span_count}")
        print(f"http_llm_projection_marker={marker}")
        print(f"http_llm_projection_otel={required(settings, 'otel_output_path')}")
        print("http llm projection e2e complete")
    finally:
        stop_process(daemon, float(required(settings, "daemon_stop_timeout_seconds")))
    return 0


def parse_args() -> argparse.Namespace:
    repo = repo_root()
    case_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument(
        "--config",
        default=str(repo / "tests/payload/http-local/operator.conf"),
    )
    parser.add_argument("--workload-config", default=str(case_dir / "workload.conf"))
    return parser.parse_args()


def workload_argv(workload: Path, settings: dict[str, str]) -> list[str]:
    return [
        sys.executable,
        str(workload),
        "--model",
        required(settings, "model"),
        "--marker",
        required(settings, "marker"),
        "--path",
        required(settings, "path"),
        "--bind-host",
        required(settings, "bind_host"),
        "--bind-port",
        required(settings, "bind_port"),
        "--response-text",
        required(settings, "response_text"),
        "--timeout-seconds",
        required(settings, "workload_timeout_seconds"),
        "--request-write-mode",
        required(settings, "request_write_mode"),
        "--host-header",
        required(settings, "host_header"),
        "--content-type",
        required(settings, "content_type"),
        "--response-read-chunk-bytes",
        required(settings, "response_read_chunk_bytes"),
        "--request-padding-bytes",
        required(settings, "request_padding_bytes"),
    ]


def require_http_llm_span(document: dict, marker: str, model: str) -> None:
    for span in otel_spans(document):
        attrs = otel_attrs(span)
        if attrs.get("actrail.action.kind") != "llm.request":
            continue
        if attrs.get("payload.source_boundary") != "Syscall":
            continue
        if attrs.get("url.scheme") != "http":
            continue
        if attrs.get("llm.request.model") != model:
            continue
        if (
            attrs.get("llm.request.payload_bytes")
            and attrs.get("llm.request.raw_payload_bytes")
            and attrs.get("llm.request.content_state") == "canonical_blocks"
            and attrs.get("llm.request.canonical_body_hash")
            and attrs.get("llm.request.block_count")
            and marker in attrs.get("llm.request.message_preview", "")
            and not attrs.get("llm.request.body_json")
            and not attrs.get("llm.request.body_text")
            and not attrs.get("http.request.body_text")
            and not attrs.get("http.request.body_json")
        ):
            return
    raise RuntimeError("OTEL export did not contain the expected HTTP llm.request span")


def parse_workload_pid(output: str) -> int:
    match = re.search(r"^workload_pid=(\d+)$", output, re.MULTILINE)
    if not match:
        raise RuntimeError("workload output did not contain workload_pid")
    return int(match.group(1))


def trace_summary(actrailviewer: Path, config: Path | None, trace_id: int) -> str:
    return run_checked(
        actrail_command(actrailviewer, config, "summary", "--trace-id", str(trace_id)),
        echo=False,
    )


def require_launch_root_pid(summary: str, workload_pid: int) -> None:
    expected = f"root_pid={workload_pid}"
    if expected not in summary:
        raise RuntimeError(
            f"launch trace root pid did not match workload pid {workload_pid}\n{summary}"
        )


def require_complete_outbound_socket_payload_rows(payloads: str) -> int:
    count = 0
    for line in payloads.splitlines():
        if not re.match(r"^\s*payload-\d+\s+", line):
            continue
        if "outbound" not in line or "Syscall" not in line or "socket-syscall" not in line:
            continue
        if "Truncated" in line or "success" not in line:
            raise RuntimeError(f"outbound socket payload row is not complete/successful: {line}")
        count += 1
    if count == 0:
        raise RuntimeError("no complete outbound Syscall socket-syscall payload rows found")
    return count


def resolve_path(raw: str, repo: Path) -> Path:
    path = Path(raw)
    return path if path.is_absolute() else repo / path


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"HTTP LLM projection e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
