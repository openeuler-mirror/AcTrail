#!/usr/bin/env python3
"""Agent trace case for local DT_NEEDED and dlsym TLS payload capture."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import sys
import time
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import (  # noqa: E402
    actrail_command,
    clean_configured_paths,
    launch_and_parse_trace,
    read_config,
    repo_root,
    require_binary,
    require_root,
    required,
    run_checked,
    start_daemon,
    stop_process,
)


@dataclass(frozen=True)
class DynamicTlsWorkload:
    name: str
    argv: list[str]
    reply: str
    payload: str
    expected_stdout_fragment: str


def main() -> int:
    args = parse_args()
    require_root()
    gcc = require_tool("gcc")
    readelf = require_tool("readelf")
    repo = repo_root()
    case_dir = Path(__file__).resolve().parent
    workload = read_config(Path(args.workload_config))
    bin_dir = repo / args.bin_dir
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
    build_dir = repo / required(workload, "build_dir")
    build_dir.mkdir(parents=True, exist_ok=True)
    binaries = build_workloads(gcc, readelf, case_dir, build_dir)
    config = Path(args.config)
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
        for item in dynamic_workloads(binaries, workload):
            run_dynamic_trace(item, workload, actrailctl, actrailviewer, config)
        print("Dynamic TLS local agent trace e2e complete")
    finally:
        stop_process(daemon, float(required(workload, "daemon_stop_timeout_seconds")))
    return 0


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


def parse_args() -> argparse.Namespace:
    case_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--config", default=str(case_dir / "operator.conf"))
    parser.add_argument("--workload-config", default=str(case_dir / "workload.conf"))
    return parser.parse_args()


def require_tool(name: str) -> str:
    path = shutil.which(name)
    if path is None:
        raise RuntimeError(f"missing required tool: {name}")
    return path


def build_workloads(gcc: str, readelf: str, case_dir: Path, build_dir: Path) -> dict[str, Path]:
    src = case_dir / "src"
    libssl = build_dir / "libssl.so"
    needed_openssl = build_dir / "needed-openssl"
    resolver_openssl = build_dir / "resolver-openssl"
    compile_shared(gcc, src / "fake_ssl.c", libssl, "libssl.so")
    compile_needed(gcc, src / "needed.c", needed_openssl, build_dir, "libssl.so")
    run_checked(
        [gcc, "-Wall", "-Wextra", "-O2", "-o", str(resolver_openssl), str(src / "resolver.c"), "-ldl"],
        echo=False,
    )
    require_needed(readelf, needed_openssl, "libssl.so")
    return {
        "libssl": libssl,
        "needed_openssl": needed_openssl,
        "resolver_openssl": resolver_openssl,
    }


def compile_shared(gcc: str, source: Path, output: Path, soname: str) -> None:
    run_checked(
        [
            gcc,
            "-Wall",
            "-Wextra",
            "-O2",
            "-shared",
            "-fPIC",
            "-Wl,-soname," + soname,
            "-o",
            str(output),
            str(source),
        ],
        echo=False,
    )


def compile_needed(gcc: str, source: Path, output: Path, build_dir: Path, library: str) -> None:
    run_checked(
        [
            gcc,
            "-Wall",
            "-Wextra",
            "-O2",
            "-o",
            str(output),
            str(source),
            "-L",
            str(build_dir),
            "-Wl,-rpath,$ORIGIN",
            "-l:" + library,
        ],
        echo=False,
    )


def require_needed(readelf: str, binary: Path, library: str) -> None:
    output = run_checked([readelf, "-d", str(binary)], echo=False)
    if f"Shared library: [{library}]" not in output:
        raise RuntimeError(f"{binary} is not linked with DT_NEEDED {library}")


def dynamic_workloads(
    binaries: dict[str, Path],
    workload: dict[str, str],
) -> list[DynamicTlsWorkload]:
    return [
        DynamicTlsWorkload(
            name="dt-needed-openssl-child-exec",
            argv=[
                "/bin/sh",
                "-c",
                'ACTRAIL_DYNAMIC_TLS_REPLY="$1" exec "$2" "$3"',
                "dynamic-tls",
                required(workload, "needed_openssl_reply"),
                str(binaries["needed_openssl"]),
                required(workload, "needed_openssl_payload"),
            ],
            reply=required(workload, "needed_openssl_reply"),
            payload=required(workload, "needed_openssl_payload"),
            expected_stdout_fragment="dynamic-needed-reply=",
        ),
        DynamicTlsWorkload(
            name="dlsym-openssl-direct",
            argv=[
                str(binaries["resolver_openssl"]),
                str(binaries["libssl"]),
                required(workload, "dlsym_openssl_payload"),
            ],
            reply=required(workload, "dlsym_openssl_reply"),
            payload=required(workload, "dlsym_openssl_payload"),
            expected_stdout_fragment="dynamic-dlsym-reply=",
        ),
    ]


def run_dynamic_trace(
    item: DynamicTlsWorkload,
    workload: dict[str, str],
    actrailctl: Path,
    actrailviewer: Path,
    config: Path,
) -> None:
    env = os.environ.copy()
    env["ACTRAIL_DYNAMIC_TLS_REPLY"] = item.reply
    env["ACTRAIL_DYNAMIC_TLS_POST_PAYLOAD_SLEEP_MS"] = required(
        workload,
        "post_payload_sleep_ms",
    )
    trace_id, output = launch_and_parse_trace_with_env(
        actrailctl,
        config,
        "agent-dynamic-tls-" + item.name,
        item.argv,
        float(required(workload, "launch_timeout_seconds")),
        env,
    )
    if item.expected_stdout_fragment + item.reply not in output:
        raise RuntimeError(f"{item.name} output missed expected reply marker")
    segments = wait_for_tls_segments(
        actrailctl,
        actrailviewer,
        config,
        trace_id,
        int(required(workload, "drain_attempts")),
        float(required(workload, "drain_sleep_seconds")),
        required(workload, "payload_head"),
    )
    require_payload_exchange(segments, item.name)
    require_payload_text(
        actrailviewer,
        config,
        trace_id,
        segments,
        item.payload,
        item.reply,
        int(required(workload, "payload_fetch_limit")),
    )
    print(f"dynamic_tls_{item.name}_trace_id={trace_id}")
    print(f"dynamic_tls_{item.name}_payload_segments={len(segments)}")


def launch_and_parse_trace_with_env(
    actrailctl: Path,
    config: Path,
    name: str,
    argv: list[str],
    timeout_sec: float,
    env: dict[str, str],
) -> tuple[int, str]:
    command = actrail_command(
        actrailctl,
        config,
        "launch",
        "--name",
        name,
        "--",
        *argv,
    )
    import re
    import subprocess

    result = subprocess.run(
        command,
        text=True,
        capture_output=True,
        check=False,
        timeout=timeout_sec,
        env=env,
    )
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
    return int(match.group(1)), f"{result.stdout}{result.stderr}"


def wait_for_tls_segments(
    actrailctl: Path,
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    attempts: int,
    sleep_sec: float,
    head: str,
) -> list[dict]:
    import time

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
        if tls_directions(segments) == {"outbound", "inbound"}:
            print(f"viewer_payloads_json_bytes={len(output.encode('utf-8'))}", flush=True)
            return segments
        time.sleep(sleep_sec)
    raise RuntimeError("viewer payload output missed dynamic TLS outbound/inbound payloads")


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
        if segment.get("symbol") not in {"SSL_write", "SSL_read"}:
            continue
        matched.append(segment)
    return matched


def tls_directions(segments: list[dict]) -> set[str]:
    return {segment["direction"] for segment in segments}


def require_payload_exchange(segments: list[dict], workload_name: str) -> None:
    for segment in segments:
        if segment.get("content_state") != "Plaintext":
            raise RuntimeError(f"{workload_name} payload is not plaintext: {segment}")
        if segment.get("truncation") != "Complete":
            raise RuntimeError(f"{workload_name} payload is truncated: {segment}")
        if segment.get("operation_completion_state") != "success":
            raise RuntimeError(f"{workload_name} payload is not successful: {segment}")
        if segment.get("operation_captured_size") != segment.get("operation_original_size"):
            raise RuntimeError(f"{workload_name} payload is partial: {segment}")


def require_payload_text(
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    segments: list[dict],
    outbound: str,
    inbound: str,
    limit: int,
) -> None:
    observed_outbound = False
    observed_inbound = False
    for segment in segments[:limit]:
        text = run_checked(
            [
                str(actrailviewer),
                "--config",
                str(config),
                "payload",
                "--trace-id",
                str(trace_id),
                "--segment-id",
                str(segment["segment_id"]),
                "--format",
                "text",
            ],
            echo=False,
        )
        if segment.get("direction") == "outbound" and outbound in text:
            observed_outbound = True
        if segment.get("direction") == "inbound" and inbound in text:
            observed_inbound = True
    if not observed_outbound:
        raise RuntimeError("dynamic TLS outbound payload text was not captured")
    if not observed_inbound:
        raise RuntimeError("dynamic TLS inbound payload text was not captured")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"Dynamic TLS agent trace e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
