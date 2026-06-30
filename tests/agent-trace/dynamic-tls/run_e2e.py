#!/usr/bin/env python3
"""Agent trace case for local DT_NEEDED and dlsym TLS payload capture."""

from __future__ import annotations

import argparse
import json
import os
import platform
import re
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


TLS_SYMBOL_SSL_WRITE = "SSL_write"
TLS_SYMBOL_SSL_WRITE_EX2 = "SSL_write_ex2"
TLS_SYMBOL_SSL_READ = "SSL_read"
TLS_SYMBOL_SSL_READ_EX = "SSL_read_ex"
EXPECTED_OPENSSL_TLS_SYMBOLS = (
    TLS_SYMBOL_SSL_WRITE,
    TLS_SYMBOL_SSL_WRITE_EX2,
    TLS_SYMBOL_SSL_READ,
)
CAPTURED_OPENSSL_TLS_SYMBOLS = {
    TLS_SYMBOL_SSL_WRITE,
    TLS_SYMBOL_SSL_WRITE_EX2,
    TLS_SYMBOL_SSL_READ,
    TLS_SYMBOL_SSL_READ_EX,
}


@dataclass(frozen=True)
class DynamicTlsWorkload:
    name: str
    argv: list[str]
    reply: str
    payload: str
    expected_symbols: tuple[str, ...]
    expected_stdout_fragment: str


def main() -> int:
    args = parse_args()
    require_root()
    gcc = require_tool("gcc")
    objdump = require_tool("objdump")
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
    binaries = build_workloads(gcc, objdump, readelf, case_dir, build_dir)
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


def build_workloads(
    gcc: str,
    objdump: str,
    readelf: str,
    case_dir: Path,
    build_dir: Path,
) -> dict[str, Path]:
    src = case_dir / "src"
    libssl = build_dir / "libssl.so"
    needed_openssl = build_dir / "needed-openssl"
    resolver_openssl = build_dir / "resolver-openssl"
    executable_jcc = build_dir / "executable-jcc-openssl"
    executable_aarch64_branch = build_dir / "executable-aarch64-branch-openssl"
    compile_shared(gcc, src / "fake_ssl.c", libssl, "libssl.so")
    compile_needed(gcc, src / "needed.c", needed_openssl, build_dir, "libssl.so")
    run_checked(
        [gcc, "-Wall", "-Wextra", "-O2", "-o", str(resolver_openssl), str(src / "resolver.c"), "-ldl"],
        echo=False,
    )
    require_needed(readelf, needed_openssl, "libssl.so")
    binaries = {
        "libssl": libssl,
        "needed_openssl": needed_openssl,
        "resolver_openssl": resolver_openssl,
    }
    if is_x86_64_host():
        compile_executable(gcc, src / "executable_jcc.c", executable_jcc)
        require_ssl_write_short_jcc(objdump, executable_jcc)
        binaries["executable_jcc_openssl"] = executable_jcc
    if is_aarch64_host():
        compile_executable(
            gcc,
            src / "executable_aarch64_branch.c",
            executable_aarch64_branch,
        )
        require_ssl_write_aarch64_external_b_cond(objdump, executable_aarch64_branch)
        binaries["executable_aarch64_branch_openssl"] = executable_aarch64_branch
    return binaries


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


def compile_executable(gcc: str, source: Path, output: Path) -> None:
    run_checked(
        [
            gcc,
            "-Wall",
            "-Wextra",
            "-O2",
            "-rdynamic",
            "-o",
            str(output),
            str(source),
        ],
        echo=False,
    )


def require_needed(readelf: str, binary: Path, library: str) -> None:
    output = run_checked([readelf, "-d", str(binary)], echo=False)
    if f"Shared library: [{library}]" not in output:
        raise RuntimeError(f"{binary} is not linked with DT_NEEDED {library}")


def require_ssl_write_short_jcc(objdump: str, binary: Path) -> None:
    block = objdump_function_block(objdump, binary, TLS_SYMBOL_SSL_WRITE)
    match = re.search(
        rf"\b78\s+[0-9a-f]{{2}}\s+js\b.*<{TLS_SYMBOL_SSL_WRITE}\+0x([0-9a-f]+)>",
        block,
    )
    if match is None:
        raise RuntimeError(f"{binary} SSL_write does not contain expected short JS opcode 0x78")
    target_offset = int(match.group(1), 16)
    if target_offset < 12:
        raise RuntimeError(
            f"{binary} SSL_write short JS target is inside the patch window: +0x{target_offset:x}"
        )


def require_ssl_write_aarch64_external_b_cond(objdump: str, binary: Path) -> None:
    output = run_checked([objdump, "-d", str(binary)], echo=False)
    symbol = re.search(rf"\n?([0-9a-f]+) <{TLS_SYMBOL_SSL_WRITE}>:", output)
    if symbol is None:
        raise RuntimeError(f"{binary} objdump output is missing {TLS_SYMBOL_SSL_WRITE}")
    function_start = int(symbol.group(1), 16)
    block = objdump_function_block(objdump, binary, TLS_SYMBOL_SSL_WRITE, [])
    for match in re.finditer(
        rf"\n\s*([0-9a-f]+):\s+[0-9a-f]{{8}}\s+b\.(?:lt|mi)\s+"
        rf"[0-9a-f]+ <{TLS_SYMBOL_SSL_WRITE}\+0x([0-9a-f]+)>",
        block,
    ):
        branch_offset = int(match.group(1), 16) - function_start
        target_offset = int(match.group(2), 16)
        if branch_offset < 16 and target_offset >= 16:
            return
    raise RuntimeError(
        f"{binary} SSL_write does not contain an aarch64 b.cond in the patch window "
        "with an external target"
    )


def objdump_function_block(
    objdump: str,
    binary: Path,
    symbol: str,
    options=None,
) -> str:
    command = [objdump, "-d"]
    if options is None:
        command.append("-Mintel")
    else:
        command.extend(options)
    command.append(str(binary))
    output = run_checked(command, echo=False)
    marker = f"<{symbol}>:"
    start = output.find(marker)
    if start < 0:
        raise RuntimeError(f"{binary} objdump output is missing {symbol}")
    tail = output[start + len(marker) :]
    next_symbol = re.search(r"\n[0-9a-f]+ <[^>]+>:", tail)
    if next_symbol is not None:
        return tail[: next_symbol.start()]
    return tail


def is_x86_64_host() -> bool:
    return platform.machine() in {"x86_64", "AMD64"}


def is_aarch64_host() -> bool:
    return platform.machine() in {"aarch64", "arm64"}


def dynamic_workloads(
    binaries: dict[str, Path],
    workload: dict[str, str],
) -> list[DynamicTlsWorkload]:
    workloads = [
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
            expected_symbols=EXPECTED_OPENSSL_TLS_SYMBOLS,
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
            expected_symbols=EXPECTED_OPENSSL_TLS_SYMBOLS,
            expected_stdout_fragment="dynamic-dlsym-reply=",
        ),
    ]
    if "executable_jcc_openssl" in binaries:
        workloads.append(
            DynamicTlsWorkload(
                name="executable-openssl-short-jcc",
                argv=[
                    str(binaries["executable_jcc_openssl"]),
                    required(workload, "executable_jcc_payload"),
                ],
                reply=required(workload, "executable_jcc_reply"),
                payload=required(workload, "executable_jcc_payload"),
                expected_symbols=(TLS_SYMBOL_SSL_WRITE, TLS_SYMBOL_SSL_READ_EX),
                expected_stdout_fragment="dynamic-executable-jcc-reply=",
            )
        )
    if "executable_aarch64_branch_openssl" in binaries:
        workloads.append(
            DynamicTlsWorkload(
                name="executable-openssl-aarch64-branch",
                argv=[
                    str(binaries["executable_aarch64_branch_openssl"]),
                    required(workload, "executable_aarch64_branch_payload"),
                ],
                reply=required(workload, "executable_aarch64_branch_reply"),
                payload=required(workload, "executable_aarch64_branch_payload"),
                expected_symbols=(TLS_SYMBOL_SSL_WRITE, TLS_SYMBOL_SSL_READ_EX),
                expected_stdout_fragment="dynamic-executable-aarch64-branch-reply=",
            )
        )
    return workloads


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
        set(item.expected_symbols),
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
    expected_symbols: set[str],
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
        if tls_directions(segments) == {"outbound", "inbound"} and expected_symbols.issubset(
            tls_symbols(segments)
        ):
            print(f"viewer_payloads_json_bytes={len(output.encode('utf-8'))}", flush=True)
            return segments
        time.sleep(sleep_sec)
    raise RuntimeError(
        "viewer payload output missed dynamic TLS payloads: "
        f"expected_symbols={sorted(expected_symbols)}"
    )


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
        if segment.get("symbol") not in CAPTURED_OPENSSL_TLS_SYMBOLS:
            continue
        matched.append(segment)
    return matched


def tls_directions(segments: list[dict]) -> set[str]:
    return {segment["direction"] for segment in segments}


def tls_symbols(segments: list[dict]) -> set[str]:
    return {segment["symbol"] for segment in segments}


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
