#!/usr/bin/env python3
"""Agent trace case for TLS flow summary reset after a bounded HTTP/1 body."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import (  # noqa: E402
    actrail_command,
    clean_configured_paths,
    read_config,
    repo_root,
    require_binary,
    require_root,
    required,
    run_checked,
    start_daemon,
    stop_process,
)

TLS_SYMBOLS = {"SSL_write", "SSL_write_ex2", "SSL_read", "SSL_read_ex"}


def main() -> int:
    args = parse_args()
    require_root()
    gcc = require_tool("gcc")
    readelf = require_tool("readelf")
    repo = repo_root()
    case_dir = Path(__file__).resolve().parent
    dynamic_src = case_dir.parent / "dynamic-tls" / "src"
    template_config = case_dir.parent / "dynamic-tls" / "operator.conf"
    workload = read_config(Path(args.workload_config))
    bin_dir = repo / args.bin_dir
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
    build_dir = repo / required(workload, "build_dir")
    build_dir.mkdir(parents=True, exist_ok=True)
    target = build_workload(gcc, readelf, case_dir / "src", dynamic_src, build_dir)
    config = Path(args.config) if args.config else build_dir / "operator.conf"
    render_operator_config(template_config, Path(args.config_patch), config)
    set_agent_invocation_commands(config, target)
    clean_configured_paths(actrailctl, config)
    daemon = start_daemon(
        actraild,
        config,
        float(required(workload, "daemon_ready_timeout_seconds")),
    )
    try:
        wait_for_tls_sync_ready(
            actrailctl,
            config,
            int(required(workload, "drain_attempts")),
            float(required(workload, "drain_sleep_seconds")),
        )
        trace_id, output = launch_target(
            actrailctl,
            config,
            target,
            required(workload, "request"),
            required(workload, "text_marker"),
            required(workload, "post_payload_sleep_ms"),
            float(required(workload, "launch_timeout_seconds")),
        )
        if "tls-flow-reset-second=" not in output or required(workload, "text_marker") not in output:
            raise RuntimeError("TLS flow reset workload output missed second response marker")
        segments = wait_for_flow_reset_segments(
            actrailctl,
            actrailviewer,
            config,
            trace_id,
            int(required(workload, "drain_attempts")),
            float(required(workload, "drain_sleep_seconds")),
            required(workload, "payload_head"),
        )
        print(f"tls_flow_reset_trace_id={trace_id}")
        print(f"tls_flow_reset_payload_segments={len(segments)}")
        print("TLS flow reset agent trace e2e complete")
    finally:
        stop_process(daemon, float(required(workload, "daemon_stop_timeout_seconds")))
    return 0


def parse_args() -> argparse.Namespace:
    case_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--config")
    parser.add_argument("--config-patch", default=str(case_dir / "operator.patch.conf"))
    parser.add_argument("--workload-config", default=str(case_dir / "workload.conf"))
    return parser.parse_args()


def require_tool(name: str) -> str:
    path = shutil.which(name)
    if path is None:
        raise RuntimeError(f"missing required tool: {name}")
    return path


def build_workload(
    gcc: str,
    readelf: str,
    src: Path,
    dynamic_src: Path,
    build_dir: Path,
) -> Path:
    libssl = build_dir / "libssl.so"
    target = build_dir / "tls-flow-reset-target"
    run_checked(
        [
            gcc,
            "-Wall",
            "-Wextra",
            "-O2",
            "-shared",
            "-fPIC",
            "-Wl,-soname,libssl.so",
            "-o",
            str(libssl),
            str(src / "fake_ssl.c"),
        ],
        echo=False,
    )
    run_checked(
        [
            gcc,
            "-Wall",
            "-Wextra",
            "-O2",
            "-o",
            str(target),
            "-I",
            str(dynamic_src),
            str(src / "target.c"),
            "-L",
            str(build_dir),
            "-Wl,-rpath,$ORIGIN",
            "-l:libssl.so",
        ],
        echo=False,
    )
    dynamic = run_checked([readelf, "-d", str(target)], echo=False)
    if "Shared library: [libssl.so]" not in dynamic:
        raise RuntimeError(f"{target} is not linked with DT_NEEDED libssl.so")
    return target


def render_operator_config(template: Path, patch: Path, output: Path) -> None:
    replacements = read_patch_config(patch)
    raw = template.read_text(encoding="utf-8")
    for old, new in operator_replacements(replacements).items():
        if old not in raw:
            raise RuntimeError(f"{template} does not contain expected value {old}")
        raw = raw.replace(old, new)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(raw, encoding="utf-8")


def set_agent_invocation_commands(config: Path, command: Path) -> None:
    raw = config.read_text(encoding="utf-8")
    old = 'commands = ["tls-flow-reset"]'
    new = f"commands = [{quoted(str(command))}]"
    if old not in raw:
        raise RuntimeError(f"{config} does not contain {old}")
    config.write_text(raw.replace(old, new), encoding="utf-8")


def read_patch_config(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if not separator:
            raise RuntimeError(f"invalid patch config line in {path}: {raw}")
        values[key.strip()] = value.strip()
    return values


def operator_replacements(values: dict[str, str]) -> dict[str, str]:
    required_keys = {
        "control_socket_path",
        "control_pid_file",
        "control_log_path",
        "storage_sqlite_path",
        "web_listen_addr",
        "export_directory",
        "export_otel_jsonl_path",
        "capture_profile_name",
        "payload_tls_sync_event_socket_path",
        "agent_invocation_commands",
        "enforcement_rules_path",
    }
    missing = sorted(required_keys.difference(values))
    if missing:
        raise RuntimeError(f"missing patch config keys: {', '.join(missing)}")
    return {
        '"/tmp/actrail-agent-dynamic-tls.sock"': quoted(values["control_socket_path"]),
        '"/tmp/actrail-agent-dynamic-tls.pid"': quoted(values["control_pid_file"]),
        '"/tmp/actrail-agent-dynamic-tls.log"': quoted(values["control_log_path"]),
        '"/tmp/actrail-agent-dynamic-tls.sqlite"': quoted(values["storage_sqlite_path"]),
        '"127.0.0.1:18101"': quoted(values["web_listen_addr"]),
        '"/tmp/actrail-agent-dynamic-tls-export"': quoted(values["export_directory"]),
        '"/tmp/actrail-agent-dynamic-tls-live-spans.otlp.jsonl"': quoted(
            values["export_otel_jsonl_path"]
        ),
        '"agent-dynamic-tls"': quoted(values["capture_profile_name"]),
        '"/tmp/actrail-agent-dynamic-tls-sync.sock"': quoted(
            values["payload_tls_sync_event_socket_path"]
        ),
        '["dynamic-tls"]': values["agent_invocation_commands"],
        '"/tmp/actrail-agent-dynamic-tls-enforcement.conf"': quoted(
            values["enforcement_rules_path"]
        ),
    }


def quoted(value: str) -> str:
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def wait_for_tls_sync_ready(
    actrailctl: Path,
    config: Path,
    attempts: int,
    sleep_sec: float,
) -> None:
    for _ in range(attempts):
        output = run_checked(actrail_command(actrailctl, config, "doctor"), echo=False)
        if "tls-sync" in output and "storage_ready=true" in output:
            print("tls_sync_ready=1", flush=True)
            return
        time.sleep(sleep_sec)
    raise RuntimeError("actrailctl doctor did not report tls-sync collector readiness")


def launch_target(
    actrailctl: Path,
    config: Path,
    target: Path,
    request: str,
    marker: str,
    sleep_ms: str,
    timeout_sec: float,
) -> tuple[int, str]:
    command = actrail_command(
        actrailctl,
        config,
        "launch",
        "--name",
        "agent-tls-flow-reset",
        "--",
        str(target),
        request,
    )
    env = os.environ.copy()
    env["ACTRAIL_TLS_FLOW_RESET_TEXT_MARKER"] = marker
    env["ACTRAIL_DYNAMIC_TLS_POST_PAYLOAD_SLEEP_MS"] = sleep_ms
    result = subprocess.run(
        command,
        text=True,
        capture_output=True,
        check=False,
        timeout=timeout_sec,
        env=env,
    )
    output = f"{result.stdout}{result.stderr}"
    if result.stdout:
        print(result.stdout, end="", flush=True)
    if result.stderr:
        print(result.stderr, end="", file=sys.stderr, flush=True)
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed: {' '.join(command)}\nstdout={result.stdout}\nstderr={result.stderr}"
        )
    match = re.search(r"trace trace-(\d+) entered Active", result.stdout)
    if not match:
        raise RuntimeError(f"could not parse trace id from launch output: {result.stdout}")
    return int(match.group(1)), output


def wait_for_flow_reset_segments(
    actrailctl: Path,
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    attempts: int,
    sleep_sec: float,
    head: str,
) -> list[dict]:
    for _ in range(attempts):
        run_checked(actrail_command(actrailctl, config, "list-traces"), echo=False)
        output = run_checked(
            [
                str(actrailviewer),
                "--output-format",
                "json",
                "--config",
                str(config),
                "payloads",
                "--trace-id",
                str(trace_id),
                "--head",
                head,
            ],
            echo=False,
        )
        segments = tls_segments(output)
        if has_bounded_flow_reset(segments):
            print(f"viewer_payloads_json_bytes={len(output.encode('utf-8'))}", flush=True)
            return segments
        time.sleep(sleep_sec)
    raise RuntimeError("viewer payload output missed bounded TLS flow reset evidence")


def tls_segments(output: str) -> list[dict]:
    document = json.loads(output)
    segments = document.get("payloads")
    if not isinstance(segments, list):
        raise RuntimeError("viewer payload JSON must contain a payloads list")
    matched = []
    for segment in segments:
        if not isinstance(segment, dict):
            continue
        if segment.get("source_boundary") != "TlsUserSpace":
            continue
        if segment.get("direction") not in {"outbound", "inbound"}:
            continue
        if segment.get("symbol") not in TLS_SYMBOLS:
            continue
        matched.append(segment)
    return matched


def has_bounded_flow_reset(segments: list[dict]) -> bool:
    summaries = [
        segment
        for segment in segments
        if (
            segment.get("direction") == "inbound"
            and "tls-summary;reason=binary_body;protocol=http/1.x"
            in str(segment.get("protocol_hint"))
        )
    ]
    return any(
        later_complete_inbound_payload(segments, summary)
        for summary in summaries
    )


def later_complete_inbound_payload(segments: list[dict], summary: dict) -> bool:
    stream_key = summary.get("stream_key")
    sequence = int(summary.get("sequence", -1))
    return any(
        segment.get("direction") == "inbound"
        and segment.get("stream_key") == stream_key
        and int(segment.get("sequence", -1)) > sequence
        and segment.get("truncation") == "Complete"
        and segment.get("operation_completion_state") == "success"
        and int(segment.get("captured_size", 0)) > 0
        and segment.get("protocol_hint") is None
        for segment in segments
    )


if __name__ == "__main__":
    raise SystemExit(main())
