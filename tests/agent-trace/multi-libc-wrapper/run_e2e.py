#!/usr/bin/env python3
"""Agent trace case for generic glibc wrapper to musl target TLS-sync switching."""

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
FORBIDDEN_LOADER_FRAGMENTS = (
    "Error relocating",
    "Error loading shared library",
    "error while loading shared libraries",
    "IFUNC",
    "LD_PRELOAD cannot be preloaded",
    "file too short",
)


def main() -> int:
    args = parse_args()
    require_root()
    gcc = require_tool("gcc")
    musl_gcc = require_musl_gcc()
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
    require_musl_runtime(bin_dir)
    build_dir = repo / required(workload, "build_dir")
    build_dir.mkdir(parents=True, exist_ok=True)
    runtime_dir = Path(required(workload, "musl_runtime_dir"))
    shutil.rmtree(runtime_dir, ignore_errors=True)
    binaries = build_workload(
        gcc,
        musl_gcc,
        readelf,
        case_dir / "src",
        dynamic_src,
        build_dir,
        runtime_dir,
    )
    config = Path(args.config) if args.config else build_dir / "operator.conf"
    render_operator_config(template_config, Path(args.config_patch), config)
    set_agent_invocation_commands(config, binaries["wrapper"])
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
        trace_id, output = launch_wrapper(
            actrailctl,
            config,
            binaries["wrapper"],
            binaries["target"],
            binaries["musl_source_dir"],
            runtime_dir,
            required(workload, "payload"),
            required(workload, "reply"),
            float(required(workload, "launch_timeout_seconds")),
        )
        require_no_loader_errors(output)
        for marker in (
            "multi_libc_wrapper_helpers=7",
            "multi_libc_target_runtime=musl",
            "multi-libc-target-reply=" + required(workload, "reply"),
            "libactrail_tls_payload_probe_sync-musl.so",
        ):
            if marker not in output:
                raise RuntimeError(f"multi-libc wrapper output missed marker: {marker}")
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
        print(f"multi_libc_wrapper_trace_id={trace_id}")
        print(f"multi_libc_wrapper_payload_segments={len(segments)}")
        print("multi_libc_wrapper_loader_error_count=0")
        print("multi-libc wrapper agent trace e2e complete")
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


def require_musl_gcc() -> str:
    for name in ("x86_64-linux-musl-gcc", "musl-gcc"):
        path = shutil.which(name)
        if path is not None:
            return path
    raise RuntimeError("missing musl-gcc; install musl-tools before running multi-libc wrapper E2E")


def require_musl_runtime(bin_dir: Path) -> None:
    path = bin_dir / "libactrail_tls_payload_probe_sync-musl.so"
    if not path.is_file():
        raise RuntimeError(
            f"missing {path}; run scripts/build-tls-sync-runtimes.sh before multi-libc wrapper E2E"
        )


def build_workload(
    gcc: str,
    musl_gcc: str,
    readelf: str,
    src: Path,
    dynamic_src: Path,
    build_dir: Path,
    runtime_dir: Path,
) -> dict[str, Path]:
    wrapper = build_dir / "generic-wrapper"
    target = build_dir / "renamed-musl-agent"
    musl_source_dir = build_dir / "bundled-musl-lib"
    musl_source_dir.mkdir(parents=True, exist_ok=True)
    build_musl_lib_dir(musl_gcc, dynamic_src, musl_source_dir)
    run_checked(
        [
            gcc,
            "-Wall",
            "-Wextra",
            "-O2",
            "-o",
            str(wrapper),
            str(src / "wrapper_launcher.c"),
        ],
        echo=False,
    )
    run_checked(
        [
            musl_gcc,
            "-Wall",
            "-Wextra",
            "-O2",
            "-o",
            str(target),
            str(src / "musl_target.c"),
            "-L",
            str(musl_source_dir),
            "-Wl,--dynamic-linker," + str(runtime_dir / "ld-musl-x86_64.so.1"),
            "-l:libssl.so",
        ],
        echo=False,
    )
    program_headers = run_checked([readelf, "-l", str(target)], echo=False)
    expected_interpreter = str(runtime_dir / "ld-musl-x86_64.so.1")
    if expected_interpreter not in program_headers:
        raise RuntimeError(f"{target} PT_INTERP is not {expected_interpreter}")
    dynamic = run_checked([readelf, "-d", str(target)], echo=False)
    if "Shared library: [libssl.so]" not in dynamic:
        raise RuntimeError(f"{target} is not linked with DT_NEEDED libssl.so")
    return {
        "wrapper": wrapper,
        "target": target,
        "musl_source_dir": musl_source_dir,
    }


def build_musl_lib_dir(musl_gcc: str, dynamic_src: Path, output: Path) -> None:
    run_checked(
        [
            musl_gcc,
            "-Wall",
            "-Wextra",
            "-O2",
            "-shared",
            "-fPIC",
            "-Wl,-soname,libssl.so",
            "-o",
            str(output / "libssl.so"),
            str(dynamic_src / "fake_ssl.c"),
        ],
        echo=False,
    )
    libc = Path("/usr/lib/x86_64-linux-musl/libc.so")
    if not libc.is_file():
        raise RuntimeError(f"missing musl libc runtime: {libc}")
    for name in ("ld-musl-x86_64.so.1", "libc.musl-x86_64.so.1", "libc.so"):
        shutil.copy2(libc, output / name)
    (output / "libgcc_s.so.1").write_text("not an elf\n", encoding="utf-8")


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
    old = 'commands = ["multi-libc-wrapper"]'
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


def launch_wrapper(
    actrailctl: Path,
    config: Path,
    wrapper: Path,
    target: Path,
    musl_source_dir: Path,
    runtime_dir: Path,
    payload: str,
    reply: str,
    timeout_sec: float,
) -> tuple[int, str]:
    env = os.environ.copy()
    env["ACTRAIL_MULTI_LIBC_MUSL_SOURCE_DIR"] = str(musl_source_dir)
    env["ACTRAIL_MULTI_LIBC_MUSL_RUNTIME_DIR"] = str(runtime_dir)
    env["ACTRAIL_DYNAMIC_TLS_REPLY"] = reply
    command = actrail_command(
        actrailctl,
        config,
        "launch",
        "--name",
        "agent-multi-libc-wrapper",
        "--",
        str(wrapper),
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


def require_no_loader_errors(output: str) -> None:
    matched = [fragment for fragment in FORBIDDEN_LOADER_FRAGMENTS if fragment in output]
    if matched:
        raise RuntimeError(f"loader error fragments found: {matched}")


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
            print(f"viewer_payloads_json_bytes={len(output.encode('utf-8'))}", flush=True)
            return segments
        time.sleep(sleep_sec)
    raise RuntimeError("viewer payload output missed multi-libc musl TLS payloads")


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
        if segment.get("operation_completion_state") != "success":
            raise RuntimeError(f"TLS payload did not complete successfully: {segment}")
        if segment.get("truncation") == "Truncated":
            raise RuntimeError(f"TLS payload was truncated: {segment}")
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
        raise RuntimeError("multi-libc outbound payload text was not captured")
    if not observed_inbound:
        raise RuntimeError("multi-libc inbound payload text was not captured")


if __name__ == "__main__":
    raise SystemExit(main())
