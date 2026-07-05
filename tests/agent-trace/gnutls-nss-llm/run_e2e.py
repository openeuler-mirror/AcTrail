#!/usr/bin/env python3
"""Agent trace case for controlled GnuTLS and NSS/NSPR LLM payload capture."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import (  # noqa: E402
    actrail_command,
    clean_configured_paths,
    read_config,
    render_config,
    repo_root,
    require_binary,
    require_complete_llm_exchange,
    require_llm_exchange_graph,
    require_root,
    required,
    run_checked,
    start_daemon,
    stop_process,
    wait_for_llm_exchange_actions,
)


@dataclass(frozen=True)
class LegacyTlsWorkload:
    name: str
    library: str
    resolver: str
    library_path: Path
    binary: Path
    expected_symbols: tuple[str, ...]
    expected_direction_symbols: tuple[tuple[str, str], ...]
    expected_stdout: str
    web_port: int


def main() -> int:
    args = parse_args()
    require_root()
    gcc = require_tool("gcc")
    repo = repo_root()
    case_dir = Path(__file__).resolve().parent
    workload = read_config(Path(args.workload_config))
    bin_dir = repo / args.bin_dir
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
    build_dir = repo / required(workload, "build_dir")
    build_dir.mkdir(parents=True, exist_ok=True)
    workloads = build_workloads(gcc, case_dir, build_dir)
    for item in workloads:
        run_workload(
            item,
            workload,
            args.config,
            build_dir,
            actraild,
            actrailctl,
            actrailviewer,
        )
    print("GnuTLS/NSS controlled LLM agent trace e2e complete")
    return 0


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


def build_workloads(gcc: str, case_dir: Path, build_dir: Path) -> list[LegacyTlsWorkload]:
    src = case_dir / "src"
    libgnutls = build_dir / "libgnutls.so"
    libnspr = build_dir / "libnspr4.so"
    gnutls_binary = build_dir / "legacy-tls-llm-locator-gnutls"
    nss_binary = build_dir / "legacy-tls-llm-locator-nss"
    nss_sendrecv_binary = build_dir / "legacy-tls-llm-locator-nss-sendrecv"
    compile_shared(gcc, src / "fake_gnutls.c", libgnutls, "libgnutls.so")
    compile_shared(gcc, src / "fake_nspr.c", libnspr, "libnspr4.so")
    compile_locator(
        gcc,
        src / "locator.c",
        gnutls_binary,
        build_dir,
        "gnutls",
        ("ACTRAIL_USE_GNUTLS",),
    )
    compile_locator(
        gcc,
        src / "locator.c",
        nss_binary,
        build_dir,
        "nspr4",
        ("ACTRAIL_USE_NSPR",),
    )
    compile_locator(
        gcc,
        src / "locator.c",
        nss_sendrecv_binary,
        build_dir,
        "nspr4",
        ("ACTRAIL_USE_NSPR", "ACTRAIL_USE_NSPR_SEND_RECV"),
    )
    return [
        LegacyTlsWorkload(
            name="gnutls",
            library="gnutls",
            resolver="gnutls-symbols",
            library_path=libgnutls,
            binary=gnutls_binary,
            expected_symbols=("gnutls_record_send", "gnutls_record_recv"),
            expected_direction_symbols=(
                ("outbound", "gnutls_record_send"),
                ("inbound", "gnutls_record_recv"),
            ),
            expected_stdout="legacy-tls-provider=gnutls",
            web_port=18111,
        ),
        LegacyTlsWorkload(
            name="nss",
            library="nss",
            resolver="nss-nspr-symbols",
            library_path=libnspr,
            binary=nss_binary,
            expected_symbols=("PR_Write", "PR_Read"),
            expected_direction_symbols=(("outbound", "PR_Write"), ("inbound", "PR_Read")),
            expected_stdout="legacy-tls-provider=nss",
            web_port=18112,
        ),
        LegacyTlsWorkload(
            name="nss-sendrecv",
            library="nss",
            resolver="nss-nspr-symbols",
            library_path=libnspr,
            binary=nss_sendrecv_binary,
            expected_symbols=("PR_Send", "PR_Recv"),
            expected_direction_symbols=(("outbound", "PR_Send"), ("inbound", "PR_Recv")),
            expected_stdout="legacy-tls-provider=nss-sendrecv",
            web_port=18113,
        ),
    ]


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


def compile_locator(
    gcc: str,
    source: Path,
    output: Path,
    build_dir: Path,
    library: str,
    defines: tuple[str, ...],
) -> None:
    define_args = [f"-D{define}" for define in defines]
    run_checked(
        [
            gcc,
            "-Wall",
            "-Wextra",
            "-O2",
            "-o",
            str(output),
            *define_args,
            str(source),
            "-L",
            str(build_dir),
            "-Wl,-rpath,$ORIGIN",
            "-l:" + f"lib{library}.so",
        ],
        echo=False,
    )


def run_workload(
    item: LegacyTlsWorkload,
    workload: dict[str, str],
    config_template: str,
    build_dir: Path,
    actraild: Path,
    actrailctl: Path,
    actrailviewer: Path,
) -> None:
    config = build_dir / f"{item.name}-operator.conf"
    render_config(
        Path(config_template),
        config,
        {
            "__CASE_NAME__": f"agent-gnutls-nss-llm-{item.name}",
            "__WEB_PORT__": str(item.web_port),
            "__TLS_RESOLVER__": item.resolver,
            "__TLS_LIBRARY__": item.library,
            "__TLS_LIBRARY_PATH__": str(item.library_path),
        },
    )
    reject_leftover_placeholders(config)
    clean_configured_paths(actrailctl, config)
    daemon = start_daemon(
        actraild,
        config,
        float(required(workload, "daemon_ready_timeout_seconds")),
    )
    try:
        trace_id, output = launch_and_parse_trace_with_env(
            daemon,
            actrailctl,
            config,
            "agent-gnutls-nss-llm-" + item.name,
            [
                str(item.binary),
                required(workload, "model"),
                required(workload, "prompt"),
            ],
            float(required(workload, "launch_timeout_seconds")),
            float(required(workload, "daemon_stop_timeout_seconds")),
            workload_env(workload),
        )
        if item.expected_stdout not in output:
            raise RuntimeError(f"{item.name} output missed provider marker")
        segments = wait_for_tls_segments(
            actrailctl,
            actrailviewer,
            config,
            trace_id,
            int(required(workload, "drain_attempts")),
            float(required(workload, "drain_sleep_seconds")),
            required(workload, "payload_head"),
            item.library,
            set(item.expected_symbols),
            set(item.expected_direction_symbols),
        )
        require_payload_exchange(segments, item.name)
        actions = wait_for_llm_exchange_actions(
            actrailviewer,
            config,
            trace_id,
            int(required(workload, "drain_attempts")),
            float(required(workload, "drain_sleep_seconds")),
        )
        require_complete_llm_exchange(actions)
        require_complete_http_messages(actions)
        require_llm_exchange_graph(actions)
        print(f"legacy_tls_{item.name}_trace_id={trace_id}")
        print(f"legacy_tls_{item.name}_payload_segments={len(segments)}")
    finally:
        stop_process(daemon, float(required(workload, "daemon_stop_timeout_seconds")))


def workload_env(workload: dict[str, str]) -> dict[str, str]:
    response_text = required(workload, "response_text")
    body = json.dumps(
        {
            "id": "chatcmpl-legacy-tls",
            "object": "chat.completion",
            "model": required(workload, "model"),
            "choices": [
                {
                    "index": 0,
                    "message": {"role": "assistant", "content": response_text},
                    "finish_reason": "stop",
                }
            ],
            "usage": {"prompt_tokens": 7, "completion_tokens": 5, "total_tokens": 12},
        },
        separators=(",", ":"),
    )
    response = (
        "HTTP/1.1 200 OK\r\n"
        "Content-Type: application/json\r\n"
        f"Content-Length: {len(body.encode('utf-8'))}\r\n"
        "Connection: close\r\n"
        "\r\n"
        f"{body}"
    )
    env = os.environ.copy()
    env["ACTRAIL_LEGACY_TLS_LLM_RESPONSE"] = response
    env["ACTRAIL_LEGACY_TLS_LLM_PRE_PAYLOAD_SLEEP_MS"] = required(
        workload,
        "pre_payload_sleep_ms",
    )
    env["ACTRAIL_LEGACY_TLS_LLM_POST_PAYLOAD_SLEEP_MS"] = required(
        workload,
        "post_payload_sleep_ms",
    )
    return env


def reject_leftover_placeholders(config: Path) -> None:
    raw = config.read_text(encoding="utf-8")
    leftovers = sorted(set(re.findall(r"__[A-Z0-9_]+__", raw)))
    if leftovers:
        raise RuntimeError(
            f"{config} contains unresolved placeholders: {', '.join(leftovers)}"
        )


def launch_and_parse_trace_with_env(
    daemon: subprocess.Popen[str],
    actrailctl: Path,
    config: Path,
    name: str,
    argv: list[str],
    timeout_sec: float,
    stop_timeout_sec: float,
    env: dict[str, str],
) -> tuple[int, str]:
    command = actrail_command(
        actrailctl,
        config,
        "launch",
        "--name",
        name,
        "--host-ebpf",
        "required",
        "--",
        *argv,
    )
    process = subprocess.Popen(
        command,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
        env=env,
    )
    started = time.monotonic()
    deadline = started + timeout_sec
    while time.monotonic() < deadline:
        returncode = process.poll()
        if returncode is not None:
            stdout, stderr = process.communicate(timeout=stop_timeout_sec)
            break
        if daemon.poll() is not None:
            stop_launch_process_group(process, stop_timeout_sec)
            stdout, stderr = process.communicate(timeout=stop_timeout_sec)
            daemon_stdout = daemon.stdout.read() if daemon.stdout else ""
            daemon_stderr = daemon.stderr.read() if daemon.stderr else ""
            raise RuntimeError(
                "actraild exited while actrailctl launch was still running\n"
                f"elapsed_seconds={time.monotonic() - started:.1f}\n"
                f"daemon_status={daemon.returncode}\n"
                f"daemon_stdout={daemon_stdout}\n"
                f"daemon_stderr={daemon_stderr}\n"
                f"launch_stdout={stdout}\n"
                f"launch_stderr={stderr}"
            )
        time.sleep(0.1)
    else:
        stop_launch_process_group(process, stop_timeout_sec)
        stdout, stderr = process.communicate(timeout=stop_timeout_sec)
        raise RuntimeError(
            f"actrailctl launch timed out after {timeout_sec}s\n"
            f"stdout={stdout}\nstderr={stderr}"
        )
    if stdout:
        print(stdout, end="", flush=True)
    if stderr:
        print(stderr, end="", file=sys.stderr, flush=True)
    if returncode != 0:
        raise RuntimeError(
            f"command failed: {' '.join(command)}\nstdout={stdout}\nstderr={stderr}"
        )
    import re

    match = re.search(r"trace trace-(\d+) entered Active", stdout)
    if not match:
        raise RuntimeError(f"could not parse trace id from launch output: {stdout}")
    return int(match.group(1)), f"{stdout}{stderr}"


def stop_launch_process_group(process: subprocess.Popen[str], timeout_sec: float) -> None:
    try:
        os.killpg(process.pid, signal.SIGCONT)
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    if process.poll() is None:
        try:
            process.wait(timeout=timeout_sec)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGCONT)
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=timeout_sec)


def wait_for_tls_segments(
    actrailctl: Path,
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    attempts: int,
    sleep_sec: float,
    head: str,
    library: str,
    expected_symbols: set[str],
    expected_direction_symbols: set[tuple[str, str]],
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
        segments = tls_segments(output, library)
        if (
            tls_directions(segments) == {"outbound", "inbound"}
            and expected_symbols.issubset(tls_symbols(segments))
            and expected_direction_symbols.issubset(tls_direction_symbols(segments))
        ):
            print(f"viewer_payloads_json_bytes={len(output.encode('utf-8'))}", flush=True)
            return segments
        time.sleep(sleep_sec)
    raise RuntimeError(
        "viewer payload output missed legacy TLS LLM payloads: "
        f"library={library} expected_symbols={sorted(expected_symbols)}"
    )


def tls_segments(output: str, library: str) -> list[dict]:
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
        if segment.get("library") != library:
            continue
        if segment.get("direction") not in {"outbound", "inbound"}:
            continue
        matched.append(segment)
    return matched


def tls_directions(segments: list[dict]) -> set[str]:
    return {segment["direction"] for segment in segments}


def tls_symbols(segments: list[dict]) -> set[str]:
    return {segment["symbol"] for segment in segments}


def tls_direction_symbols(segments: list[dict]) -> set[tuple[str, str]]:
    return {(segment["direction"], segment["symbol"]) for segment in segments}


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


def require_complete_http_messages(actions: str) -> None:
    document = json.loads(actions)
    count = 0
    for action in document.get("actions", []):
        if action.get("kind") != "http.message":
            continue
        if action.get("completeness") == "complete" and action.get("status") == "success":
            count += 1
    if count < 2:
        raise RuntimeError(f"expected complete request and response http.message actions, got {count}")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"GnuTLS/NSS controlled LLM agent trace e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
