#!/usr/bin/env python3
"""Agent trace case for automatic Go and cgo HTTPS LLM capture."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import (  # noqa: E402
    clean_configured_paths,
    launch_and_parse_trace,
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

GO_TLS_WRITE_SYMBOL = "crypto/tls.(*Conn).Write"
GO_TLS_READ_SYMBOL = "crypto/tls.(*Conn).Read"
OPENSSL_RUNTIME_ENV = "ACTRAIL_GO_OPENSSL_E2E"


@dataclass(frozen=True)
class GoWorkload:
    name: str
    source_dir: Path
    binary: Path
    output_marker: str
    tls_library: str
    module_mode: str
    cgo_enabled: bool
    require_go_fast_plan: bool


def main() -> int:
    args = parse_args()
    require_root()
    repo = repo_root()
    case_dir = Path(__file__).resolve().parent
    workload = read_config(Path(args.workload_config))
    api_key_env = required(workload, "api_key_env")
    if not os.environ.get(api_key_env):
        raise RuntimeError(f"missing environment variable {api_key_env}")
    bin_dir = repo / args.bin_dir
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
    finder = require_binary(bin_dir, "tls-probe-point-finder")
    build_dir = repo / "target" / "agent-trace" / "go-net-http"
    runtime_config = build_dir / "operator.conf"
    build_dir.mkdir(parents=True, exist_ok=True)
    render_config(Path(args.config), runtime_config, {})
    require_generic_tls_config(runtime_config)
    workloads = build_go_workloads(case_dir, build_dir, repo, finder)
    clean_configured_paths(actrailctl, runtime_config)
    daemon = start_daemon(
        actraild,
        runtime_config,
        float(required(workload, "daemon_ready_timeout_seconds")),
    )
    try:
        for item in workloads:
            run_workload_trace(
                item,
                workload,
                api_key_env,
                actrailctl,
                actrailviewer,
                runtime_config,
            )
        print("Go automatic TLS agent trace e2e complete")
    finally:
        stop_process(daemon, float(required(workload, "daemon_stop_timeout_seconds")))
    return 0


def parse_args() -> argparse.Namespace:
    case_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--config", default=str(case_dir / "operator.conf"))
    parser.add_argument("--workload-config", default=str(case_dir / "workload.conf"))
    return parser.parse_args()


def build_go_workloads(
    case_dir: Path,
    build_dir: Path,
    repo: Path,
    finder: Path,
) -> list[GoWorkload]:
    workloads = [
        GoWorkload(
            name="stdlib",
            source_dir=case_dir / "workloads" / "stdlib",
            binary=build_dir / "go-stdlib-workload",
            output_marker="go-stdlib-status=200",
            tls_library="go",
            module_mode="off",
            cgo_enabled=False,
            require_go_fast_plan=True,
        ),
        GoWorkload(
            name="library",
            source_dir=case_dir / "workloads" / "library",
            binary=build_dir / "go-library-workload",
            output_marker="go-library-status=200",
            tls_library="go",
            module_mode="on",
            cgo_enabled=False,
            require_go_fast_plan=True,
        ),
    ]
    for item in workloads:
        build_go_workload(item, repo)
        if item.require_go_fast_plan:
            require_auto_go_fast_plan(finder, item.binary)
    openssl = optional_openssl_workload(case_dir, build_dir, repo, finder)
    if openssl is not None:
        workloads.append(openssl)
    return workloads


def optional_openssl_workload(
    case_dir: Path,
    build_dir: Path,
    repo: Path,
    finder: Path,
) -> GoWorkload | None:
    if not openssl_build_available():
        print("skip go-openssl workload: pkg-config openssl is unavailable", flush=True)
        return None
    item = GoWorkload(
        name="openssl",
        source_dir=case_dir / "workloads" / "openssl",
        binary=build_dir / "go-openssl-workload",
        output_marker="go-openssl-status=200",
        tls_library="openssl",
        module_mode="on",
        cgo_enabled=True,
        require_go_fast_plan=False,
    )
    build_go_workload(item, repo)
    if not auto_fast_plan_is_native_openssl(finder, item.binary):
        print(
            "skip go-openssl workload: auto fast plan did not resolve OpenSSL shared-library hooks",
            flush=True,
        )
        return None
    print("go_openssl_fast_plan=ok", flush=True)
    if os.environ.get(OPENSSL_RUNTIME_ENV) != "1":
        print(
            f"skip go-openssl runtime capture: set {OPENSSL_RUNTIME_ENV}=1 to run cgo/OpenSSL provider traffic",
            flush=True,
        )
        return None
    return item


def openssl_build_available() -> bool:
    if shutil.which("pkg-config") is None:
        return False
    go = shutil.which("go")
    if go is None:
        raise RuntimeError("go executable is required for go-net-http E2E")
    result = subprocess.run(
        [go, "env", "CGO_ENABLED"],
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0 or result.stdout.strip() != "1":
        return False
    return subprocess.run(["pkg-config", "--exists", "openssl"], check=False).returncode == 0


def build_go_workload(item: GoWorkload, repo: Path) -> None:
    go = shutil.which("go")
    if go is None:
        raise RuntimeError("go executable is required for go-net-http E2E")
    item.binary.parent.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env["GO111MODULE"] = item.module_mode
    env["GOCACHE"] = str(repo / "target" / "agent-trace" / "go-build-cache")
    env["CGO_ENABLED"] = "1" if item.cgo_enabled else "0"
    Path(env["GOCACHE"]).mkdir(parents=True, exist_ok=True)
    command = [go, "build", "-ldflags=-s -w", "-o", str(item.binary), "."]
    result = subprocess.run(
        command,
        text=True,
        capture_output=True,
        check=False,
        cwd=item.source_dir,
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


def require_auto_go_fast_plan(finder: Path, workload_binary: Path) -> None:
    output = run_checked([str(finder), "fast", str(workload_binary)], echo=False)
    required_fragments = [
        "provider = go",
        "resolver = go-pclntab",
        GO_TLS_WRITE_SYMBOL,
        GO_TLS_READ_SYMBOL,
        "runtime.memmove",
        "direction = inbound",
        "direction = outbound",
        "direction = control",
    ]
    for fragment in required_fragments:
        if fragment not in output:
            raise RuntimeError(f"Go auto fast probe plan missed {fragment!r}")


def auto_fast_plan_is_native_openssl(finder: Path, workload_binary: Path) -> bool:
    output = run_checked([str(finder), "fast", str(workload_binary)], echo=False)
    return (
        "provider = openssl" in output
        and "source = shared-library" in output
        and "SSL_write" in output
        and "SSL_read" in output
    )


def require_generic_tls_config(config: Path) -> None:
    raw = config.read_text(encoding="utf-8")
    forbidden = [
        "payload_tls_resolver = go-pclntab",
        "payload_tls_library = go",
        "__GO_TLS_BINARY__",
        "--provider go",
    ]
    for fragment in forbidden:
        if fragment in raw:
            raise RuntimeError(f"go-net-http config is not generic: found {fragment}")


def run_workload_trace(
    item: GoWorkload,
    workload: dict[str, str],
    api_key_env: str,
    actrailctl: Path,
    actrailviewer: Path,
    runtime_config: Path,
) -> None:
    trace_id, output = launch_and_parse_trace(
        actrailctl,
        runtime_config,
        f"agent-go-net-http-{item.name}",
        [
            str(item.binary),
            "--api-url",
            required(workload, "api_url"),
            "--api-key-env",
            api_key_env,
            "--model",
            required(workload, "model"),
            "--prompt",
            required(workload, "prompt"),
        ],
        float(required(workload, "launch_timeout_seconds")),
    )
    if item.output_marker not in output:
        raise RuntimeError(f"{item.name} workload output did not contain expected marker")
    payloads = wait_for_tls_payload(
        actrailctl,
        actrailviewer,
        runtime_config,
        trace_id,
        int(required(workload, "drain_attempts")),
        float(required(workload, "drain_sleep_seconds")),
        required(workload, "payload_head"),
        item.tls_library,
    )
    payload_count = require_tls_payload_exchange(payloads, item.tls_library)
    actions = wait_for_llm_exchange_actions(
        actrailviewer,
        runtime_config,
        trace_id,
        int(required(workload, "drain_attempts")),
        float(required(workload, "drain_sleep_seconds")),
    )
    require_complete_llm_exchange(actions)
    require_llm_exchange_graph(actions)
    print(f"go_{item.name}_trace_id={trace_id}")
    print(f"go_{item.name}_tls_payload_segments={payload_count}")


def wait_for_tls_payload(
    actrailctl: Path,
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    attempts: int,
    sleep_sec: float,
    head: str,
    library: str,
) -> str:
    for _ in range(attempts):
        run_checked([str(actrailctl), "--config", str(config), "list-traces"], echo=False)
        output = run_checked(
            [
                str(actrailviewer),
                "--output-format",
                "json",
                "payloads",
                "--config",
                str(config),
                "--trace-id",
                str(trace_id),
                "--head",
                head,
            ],
            echo=False,
        )
        if tls_payload_directions(output, library) == {"outbound", "inbound"}:
            print(
                f"viewer_payloads_json_bytes={len(output.encode('utf-8'))}",
                flush=True,
            )
            return output
        time.sleep(sleep_sec)
    raise RuntimeError(f"viewer payload output missed {library} TLS request/response payloads")


def tls_payload_directions(payloads: str, library: str) -> set[str]:
    directions: set[str] = set()
    for segment in tls_payload_segments(payloads, library):
        direction = segment.get("direction")
        if isinstance(direction, str):
            directions.add(direction)
    return directions


def require_tls_payload_exchange(payloads: str, library: str) -> int:
    segments = tls_payload_segments(payloads, library)
    outbound = [segment for segment in segments if segment.get("direction") == "outbound"]
    inbound = [segment for segment in segments if segment.get("direction") == "inbound"]
    if not outbound:
        raise RuntimeError(f"viewer payloads missed {library} TLS write payload")
    if not inbound:
        raise RuntimeError(f"viewer payloads missed {library} TLS read payload")
    return len(segments)


def tls_payload_segments(payloads: str, library: str) -> list[dict]:
    try:
        document = json.loads(payloads)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"viewer payloads output was not JSON: {error}") from error
    segments = document.get("payloads")
    if not isinstance(segments, list):
        raise RuntimeError("viewer payloads JSON must contain a payloads list")
    matched: list[dict] = []
    for segment in segments:
        if not isinstance(segment, dict):
            continue
        if segment.get("source_boundary") != "TlsUserSpace":
            continue
        if segment.get("library") != library:
            continue
        direction = segment.get("direction")
        if direction not in {"outbound", "inbound"}:
            continue
        if library == "go" and not go_symbol_matches(direction, segment.get("symbol")):
            continue
        if segment.get("content_state") != "Plaintext":
            raise RuntimeError(f"TLS payload is not plaintext: {segment}")
        if segment.get("truncation") != "Complete":
            raise RuntimeError(f"TLS payload is truncated: {segment}")
        if segment.get("operation_completion_state") != "success":
            raise RuntimeError(f"TLS payload operation is not successful: {segment}")
        if segment.get("operation_captured_size") != segment.get("operation_original_size"):
            raise RuntimeError(f"TLS payload operation is partial: {segment}")
        matched.append(segment)
    return matched


def go_symbol_matches(direction: str, symbol: object) -> bool:
    if direction == "outbound":
        return symbol == GO_TLS_WRITE_SYMBOL
    if direction == "inbound":
        return symbol == GO_TLS_READ_SYMBOL
    return False


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"Go automatic TLS agent trace e2e failed: {error}", file=sys.stderr)
        raise
