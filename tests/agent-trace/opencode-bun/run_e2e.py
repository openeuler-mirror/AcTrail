#!/usr/bin/env python3
"""Agent trace case for real opencode LLM exchange capture."""

from __future__ import annotations

import argparse
import os
import platform
import shutil
import sys
from dataclasses import dataclass
from pathlib import Path

ELF_MAGIC = b"\x7fELF"

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "runtime_tls"))
from boringssl import prepare_bun_static_boringssl_map  # noqa: E402
from common import (  # noqa: E402
    clean_configured_paths,
    emit_llm_otel_evidence,
    export_otel,
    launch_and_parse_trace,
    read_config,
    render_config,
    repo_root,
    require_binary,
    require_complete_llm_exchange,
    require_complete_payload_rows_any,
    require_otel_span,
    require_root,
    required,
    start_daemon,
    stop_process,
    wait_for_llm_exchange_actions,
    wait_for_payloads_any,
)


@dataclass(frozen=True)
class OpencodeTlsRuntime:
    binary: Path
    resolver: str
    pattern_path: str


def main() -> int:
    args = parse_args()
    require_root()
    repo = repo_root()
    case_dir = Path(__file__).resolve().parent
    workload = read_config(Path(args.workload_config))
    bin_dir = repo / args.bin_dir
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
    opencode_entry = require_opencode_entry()
    configured_symbol_map = resolve_path(required(workload, "symbol_map_path"), repo)
    tls_runtime = resolve_optional_opencode_tls_runtime(
        opencode_entry,
        configured_symbol_map,
        workload,
    )
    resolved_config = Path(required(workload, "resolved_config_path"))
    render_config(
        Path(args.config_template),
        resolved_config,
        opencode_config_replacements(tls_runtime),
    )
    clean_configured_paths(actrailctl, resolved_config)
    daemon = start_daemon(
        actraild,
        resolved_config,
        float(required(workload, "daemon_ready_timeout_seconds")),
    )
    try:
        trace_id, output = launch_and_parse_trace(
            actrailctl,
            resolved_config,
            "agent-opencode-bun",
            [
                "opencode",
                "run",
                "-m",
                required(workload, "model"),
                required(workload, "prompt"),
            ],
            float(required(workload, "launch_timeout_seconds")),
        )
        if required(workload, "expected_output_fragment") not in output:
            raise RuntimeError("opencode output did not contain expected marker")
        payloads = wait_for_payloads_any(
            actrailctl,
            actrailviewer,
            resolved_config,
            trace_id,
            int(required(workload, "drain_attempts")),
            float(required(workload, "drain_sleep_seconds")),
            required(workload, "payload_head"),
            accepted_payload_fragments(tls_runtime),
        )
        payload_count = require_complete_payload_rows_any(
            payloads,
            accepted_payload_sources(tls_runtime),
            direction="outbound",
        )
        response_payload_count = require_tls_response_payloads(payloads, tls_runtime)
        actions = wait_for_llm_exchange_actions(
            actrailviewer,
            resolved_config,
            trace_id,
            int(required(workload, "drain_attempts")),
            float(required(workload, "drain_sleep_seconds")),
        )
        require_complete_llm_exchange(actions)
        otel = export_otel(
            actrailviewer,
            resolved_config,
            trace_id,
            Path(required(workload, "otel_output_path")),
        )
        request_span_count = require_otel_span(otel, "llm.request")
        response_span_count = require_otel_span(otel, "llm.response")
        emit_llm_otel_evidence(otel, int(required(workload, "evidence_text_max_chars")))
        print(f"opencode_trace_id={trace_id}")
        print(f"opencode_payload_segments={payload_count}")
        print(f"opencode_response_payload_segments={response_payload_count}")
        print(f"opencode_llm_request_spans={request_span_count}")
        print(f"opencode_llm_response_spans={response_span_count}")
        print("opencode agent trace e2e complete")
    finally:
        stop_process(daemon, float(required(workload, "daemon_stop_timeout_seconds")))
    return 0


def parse_args() -> argparse.Namespace:
    case_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--config-template", default=str(case_dir / "operator.conf"))
    parser.add_argument("--workload-config", default=str(case_dir / "workload.conf"))
    return parser.parse_args()


def require_opencode_entry() -> Path:
    entry = shutil.which("opencode")
    if entry is None:
        raise RuntimeError("opencode is not on PATH")
    return require_executable(Path(entry))


def resolve_optional_opencode_tls_runtime(
    entry: Path,
    configured_symbol_map: Path,
    workload: dict[str, str],
) -> OpencodeTlsRuntime | None:
    env_path = os.environ.get("OPENCODE_BIN_PATH")
    if env_path:
        binary = require_executable(Path(env_path))
    elif is_elf_binary(entry):
        binary = entry
    else:
        sibling = entry.parent / ".opencode"
        if not sibling.exists():
            print("opencode_tls_runtime=disabled no sibling .opencode Bun executable")
            return None
        binary = require_executable(sibling)
    if platform.machine() in {"aarch64", "x86_64"}:
        print(f"opencode_tls_runtime={binary} using built-in static BoringSSL detector")
        return OpencodeTlsRuntime(
            binary=binary,
            resolver="boringssl-static",
            pattern_path="disabled",
        )
    try:
        symbol_map, detail = prepare_bun_static_boringssl_map(
            binary,
            configured_symbol_map,
            workload,
        )
    except Exception as error:
        if env_path:
            raise
        print(f"opencode_tls_runtime=disabled {error}")
        return None
    print(f"opencode_tls_map={symbol_map} {detail}")
    return OpencodeTlsRuntime(
        binary=binary,
        resolver="bun-static-boringssl",
        pattern_path=str(symbol_map),
    )


def opencode_config_replacements(tls_runtime: OpencodeTlsRuntime | None) -> dict[str, str]:
    if tls_runtime is None:
        return {
            "__OPENCODE_TLS_ENABLED__": "false",
            "__OPENCODE_TLS_RESOLVER__": "bun-static-boringssl",
            "__OPENCODE_TLS_BINARY__": "disabled",
            "__OPENCODE_BORINGSSL_SYMBOL_MAP__": "disabled",
            "__OPENCODE_SECCOMP_NOTIFY_ENABLED__": "true",
            "__OPENCODE_TLS_REQUIRED_CAPABILITY__": "# tls-plaintext-payload disabled",
        }
    return {
        "__OPENCODE_TLS_ENABLED__": "true",
        "__OPENCODE_TLS_RESOLVER__": tls_runtime.resolver,
        "__OPENCODE_TLS_BINARY__": str(tls_runtime.binary),
        "__OPENCODE_BORINGSSL_SYMBOL_MAP__": tls_runtime.pattern_path,
        "__OPENCODE_SECCOMP_NOTIFY_ENABLED__": "true",
        "__OPENCODE_TLS_REQUIRED_CAPABILITY__": "required_capability = tls-plaintext-payload",
    }


def accepted_payload_sources(tls_runtime: OpencodeTlsRuntime | None) -> list[tuple[str, str]]:
    sources = [("Syscall", "socket-syscall")]
    if tls_runtime is not None:
        sources.insert(0, ("TlsUserSpace", "boringssl"))
    return sources


def accepted_payload_fragments(tls_runtime: OpencodeTlsRuntime | None) -> list[list[str]]:
    fragments = [
        [source, library, "outbound", "Complete", "success"]
        for source, library in accepted_payload_sources(tls_runtime)
    ]
    if tls_runtime is not None:
        fragments.insert(0, ["TlsUserSpace", "boringssl", "outbound", "inbound", "Complete", "success"])
    return fragments


def require_tls_response_payloads(payloads: str, tls_runtime: OpencodeTlsRuntime | None) -> int:
    if tls_runtime is None:
        return 0
    return require_complete_payload_rows_any(
        payloads,
        [("TlsUserSpace", "boringssl")],
        direction="inbound",
    )


def require_executable(path: Path) -> Path:
    if not path.exists() or not os.access(path, os.X_OK):
        raise RuntimeError(f"not an executable: {path}")
    return path.resolve()


def is_elf_binary(path: Path) -> bool:
    try:
        with path.open("rb") as handle:
            return handle.read(len(ELF_MAGIC)) == ELF_MAGIC
    except OSError:
        return False


def resolve_path(raw: str, repo: Path) -> Path:
    path = Path(raw)
    return path if path.is_absolute() else repo / path


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"opencode agent trace e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
