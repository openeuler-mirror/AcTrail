#!/usr/bin/env python3
"""Agent trace case for TLS dynamic plan lookup through a peer mount namespace."""

from __future__ import annotations

import argparse
import json
import os
import platform
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
from common.process import TRACE_RE  # noqa: E402


def main() -> int:
    args = parse_args()
    require_root()
    gcc = require_tool("gcc")
    repo = repo_root()
    case_dir = Path(__file__).resolve().parent
    dynamic_src = case_dir.parent / "dynamic-tls" / "src"
    workload = read_config(Path(args.workload_config))
    bin_dir = repo / args.bin_dir
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
    build_dir = repo / required(workload, "build_dir")
    build_dir.mkdir(parents=True, exist_ok=True)
    binaries = build_workload(gcc, case_dir, dynamic_src, build_dir)
    config = build_dir / "operator.conf"
    render_operator_config(case_dir.parent / "dynamic-tls" / "operator.conf", config)
    clean_configured_paths(actrailctl, config)
    mount_dir = build_dir / "runtime-root"
    prepare_mount_dir(mount_dir)
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
        trace_id, output = launch_namespaced_agent(
            actrailctl,
            config,
            binaries["wrapper"],
            mount_dir,
            binaries["target"],
            required(workload, "payload"),
            required(workload, "reply"),
            required(workload, "post_payload_sleep_ms"),
            float(required(workload, "launch_timeout_seconds")),
        )
        if expected_reply_marker() + required(workload, "reply") not in output:
            raise RuntimeError("namespaced TLS target output missed reply marker")
        segments = wait_for_tls_segments(
            actrailctl,
            actrailviewer,
            config,
            trace_id,
            int(required(workload, "drain_attempts")),
            float(required(workload, "drain_sleep_seconds")),
            required(workload, "payload_head"),
        )
        require_payload_text(
            actrailviewer,
            config,
            trace_id,
            segments,
            required(workload, "payload"),
            required(workload, "reply"),
        )
        print(f"namespaced_tls_runtime_path={mount_dir / 'agent'}")
        print(f"namespaced_tls_trace_id={trace_id}")
        print("namespaced TLS lookup agent trace e2e complete")
    finally:
        stop_process(daemon, float(required(workload, "daemon_stop_timeout_seconds")))
    return 0


def parse_args() -> argparse.Namespace:
    case_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--workload-config", default=str(case_dir / "workload.conf"))
    return parser.parse_args()


def require_tool(name: str) -> str:
    path = shutil.which(name)
    if path is None:
        raise RuntimeError(f"missing required tool: {name}")
    return path


def build_workload(gcc: str, case_dir: Path, dynamic_src: Path, build_dir: Path) -> dict[str, Path]:
    wrapper = build_dir / "namespace-wrapper"
    target = build_dir / "source-agent"
    run_checked(
        [
            gcc,
            "-Wall",
            "-Wextra",
            "-O2",
            "-o",
            str(wrapper),
            str(case_dir / "src" / "namespace_wrapper.c"),
        ],
        echo=False,
    )
    run_checked(
        [
            gcc,
            "-Wall",
            "-Wextra",
            "-O2",
            "-rdynamic",
            "-o",
            str(target),
            str(dynamic_tls_source(dynamic_src)),
        ],
        echo=False,
    )
    return {"wrapper": wrapper, "target": target}


def dynamic_tls_source(dynamic_src: Path) -> Path:
    machine = platform.machine().lower()
    if machine in {"x86_64", "amd64"}:
        return dynamic_src / "executable_jcc.c"
    if machine in {"aarch64", "arm64"}:
        return dynamic_src / "executable_aarch64_branch.c"
    raise RuntimeError(f"unsupported namespaced TLS lookup test architecture: {machine}")


def expected_reply_marker() -> str:
    machine = platform.machine().lower()
    if machine in {"x86_64", "amd64"}:
        return "dynamic-executable-jcc-reply="
    if machine in {"aarch64", "arm64"}:
        return "dynamic-executable-aarch64-branch-reply="
    raise RuntimeError(f"unsupported namespaced TLS lookup test architecture: {machine}")


def render_operator_config(template: Path, output: Path) -> None:
    raw = template.read_text(encoding="utf-8")
    raw = raw.replace("agent-dynamic-tls", "agent-namespaced-tls-lookup")
    raw = raw.replace('profile_name = "agent-dynamic-tls"', 'profile_name = "agent-ns-tls"')
    output.write_text(raw, encoding="utf-8")


def prepare_mount_dir(path: Path) -> None:
    if (path / "agent").exists():
        (path / "agent").unlink()
    path.mkdir(parents=True, exist_ok=True)


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


def launch_namespaced_agent(
    actrailctl: Path,
    config: Path,
    wrapper: Path,
    mount_dir: Path,
    target: Path,
    payload: str,
    reply: str,
    post_payload_sleep_ms: str,
    timeout_sec: float,
) -> tuple[int, str]:
    env = os.environ.copy()
    env["ACTRAIL_DYNAMIC_TLS_REPLY"] = reply
    env["ACTRAIL_DYNAMIC_TLS_POST_PAYLOAD_SLEEP_MS"] = post_payload_sleep_ms
    command = actrail_command(
        actrailctl,
        config,
        "launch",
        "--name",
        "agent-namespaced-tls-lookup",
        "--",
        str(wrapper),
        str(mount_dir),
        str(target),
        payload,
    )
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
    match = TRACE_RE.search(result.stdout)
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
            print(f"namespaced_tls_payload_segments={len(segments)}")
            return segments
        time.sleep(sleep_sec)
    raise RuntimeError("viewer payload output missed namespaced TLS payloads")


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
        if segment.get("library") != "openssl":
            continue
        if segment.get("direction") in {"outbound", "inbound"}:
            matched.append(segment)
    return matched


def tls_directions(segments: list[dict]) -> set[str]:
    return {segment["direction"] for segment in segments}


def require_payload_text(
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    segments: list[dict],
    outbound: str,
    inbound: str,
) -> None:
    observed_outbound = False
    observed_inbound = False
    for segment in segments:
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
        raise RuntimeError("namespaced TLS outbound payload text was not captured")
    if not observed_inbound:
        raise RuntimeError("namespaced TLS inbound payload text was not captured")


if __name__ == "__main__":
    raise SystemExit(main())
