#!/usr/bin/env python3
"""Agent trace case for polluted LD_LIBRARY_PATH wrapper launchers."""

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
    "error while loading shared libraries",
    "IFUNC",
    "memmove",
    "LD_PRELOAD cannot be preloaded",
    "file too short",
)


def main() -> int:
    args = parse_args()
    require_root()
    gcc = require_tool("gcc")
    readelf = require_tool("readelf")
    repo = repo_root()
    case_dir = Path(__file__).resolve().parent
    template_config = case_dir.parent / "dynamic-tls" / "operator.conf"
    dynamic_src = case_dir.parent / "dynamic-tls" / "src"
    case_src = case_dir / "src"
    workload = read_config(Path(args.workload_config))
    bin_dir = repo / args.bin_dir
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
    build_dir = repo / required(workload, "build_dir")
    build_dir.mkdir(parents=True, exist_ok=True)
    binaries = build_workload(gcc, readelf, dynamic_src, case_src, build_dir)
    poison_dir = build_poison_library_dir(build_dir)
    wrapper = binaries["wrapper_launcher"]
    config = Path(args.config) if args.config else build_dir / "operator.conf"
    render_operator_config(template_config, Path(args.config_patch), config)
    set_agent_invocation_commands(config, wrapper)
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
            wrapper,
            poison_dir,
            binaries["target_lib_dir"],
            binaries["guard_source_dir"],
            required(workload, "payload"),
            required(workload, "reply"),
            required(workload, "post_payload_sleep_ms"),
            float(required(workload, "launch_timeout_seconds")),
        )
        require_no_loader_errors(output)
        if "polluted_env_wrapper_helpers=8" not in output:
            raise RuntimeError("wrapper output missed helper completion count")
        if "polluted-env-wrapper-reply=" + required(workload, "reply") not in output:
            raise RuntimeError("wrapper output missed final TLS workload reply")
        segments = wait_for_tls_segments(
            actrailctl,
            actrailviewer,
            config,
            trace_id,
            int(required(workload, "drain_attempts")),
            float(required(workload, "drain_sleep_seconds")),
            required(workload, "payload_head"),
        )
        print(f"polluted_env_wrapper_trace_id={trace_id}")
        print("polluted_env_wrapper_helper_exit_code=0")
        print("polluted_env_wrapper_agent_exit_code=0")
        print(f"polluted_env_wrapper_payload_segments={len(segments)}")
        print("polluted_env_wrapper_loader_error_count=0")
        print("Polluted environment wrapper agent trace e2e complete")
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
    case_src: Path,
    build_dir: Path,
) -> dict[str, Path]:
    libssl = build_dir / "libssl.so"
    target_lib_dir = build_dir / "target-libs"
    guard_source_dir = build_dir / "guard-source-libs"
    wrapper_launcher = build_dir / "polluted-wrapper-launcher"
    target_lib_dir.mkdir(parents=True, exist_ok=True)
    guard_source_dir.mkdir(parents=True, exist_ok=True)
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
            "-shared",
            "-fPIC",
            "-Wl,-soname,libstdc++.so.6",
            "-o",
            str(target_lib_dir / "libstdc++.so.6"),
            str(case_src / "private_libstdcxx.c"),
        ],
        echo=False,
    )
    versioned_libgcc = guard_source_dir / "libgcc_s-versioned.so.1"
    loader_libgcc = guard_source_dir / "libgcc_s.so.1"
    shutil.copy2(find_system_libgcc(gcc), versioned_libgcc)
    if loader_libgcc.exists() or loader_libgcc.is_symlink():
        loader_libgcc.unlink()
    os.symlink(versioned_libgcc.name, loader_libgcc)
    (guard_source_dir / "libstdc++.so.6").write_text("not an elf\n", encoding="utf-8")
    run_checked(
        [
            gcc,
            "-Wall",
            "-Wextra",
            "-O2",
            "-o",
            str(wrapper_launcher),
            "-I",
            str(src),
            str(case_src / "wrapper_launcher.c"),
            "-L",
            str(build_dir),
            "-L",
            str(target_lib_dir),
            "-Wl,-rpath,$ORIGIN",
            "-l:libssl.so",
            "-l:libstdc++.so.6",
        ],
        echo=False,
    )
    output = run_checked([readelf, "-d", str(wrapper_launcher)], echo=False)
    if "Shared library: [libssl.so]" not in output:
        raise RuntimeError(f"{wrapper_launcher} is not linked with DT_NEEDED libssl.so")
    if "Shared library: [libstdc++.so.6]" not in output:
        raise RuntimeError(f"{wrapper_launcher} is not linked with DT_NEEDED libstdc++.so.6")
    return {
        "wrapper_launcher": wrapper_launcher,
        "target_lib_dir": target_lib_dir,
        "guard_source_dir": guard_source_dir,
    }


def find_system_libgcc(gcc: str) -> Path:
    result = subprocess.run(
        [gcc, "-print-file-name=libgcc_s.so.1"],
        text=True,
        capture_output=True,
        check=False,
    )
    candidate = Path(result.stdout.strip())
    if result.returncode == 0 and candidate.is_file():
        return candidate
    for directory in (
        Path("/lib/x86_64-linux-gnu"),
        Path("/usr/lib/x86_64-linux-gnu"),
        Path("/lib64"),
        Path("/usr/lib64"),
        Path("/lib"),
        Path("/usr/lib"),
    ):
        candidate = directory / "libgcc_s.so.1"
        if candidate.is_file():
            return candidate
    raise RuntimeError("could not find system libgcc_s.so.1")


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
    old = 'commands = ["polluted-env-wrapper"]'
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


def build_poison_library_dir(build_dir: Path) -> Path:
    poison_dir = build_dir / "poison-lib"
    poison_dir.mkdir(parents=True, exist_ok=True)
    (poison_dir / "libgcc_s.so.1").write_text("not an elf\n", encoding="utf-8")
    return poison_dir


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
    poison_dir: Path,
    target_lib_dir: Path,
    guard_source_dir: Path,
    payload: str,
    reply: str,
    post_payload_sleep_ms: str,
    timeout_sec: float,
) -> tuple[int, str]:
    env = os.environ.copy()
    env["ACTRAIL_POLLUTED_LIBRARY_PATH"] = str(poison_dir)
    env["ACTRAIL_DYNAMIC_TLS_REPLY"] = reply
    env["ACTRAIL_DYNAMIC_TLS_POST_PAYLOAD_SLEEP_MS"] = post_payload_sleep_ms
    env["LD_LIBRARY_PATH"] = str(target_lib_dir)
    env["TLS_PAYLOAD_SYNC_SYSTEM_LIBRARY_DIRS"] = str(guard_source_dir)
    command = actrail_command(
        actrailctl,
        config,
        "launch",
        "--name",
        "agent-polluted-env-wrapper",
        "--",
        str(wrapper),
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
    raise RuntimeError("viewer payload output missed polluted wrapper TLS payloads")


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


if __name__ == "__main__":
    raise SystemExit(main())
