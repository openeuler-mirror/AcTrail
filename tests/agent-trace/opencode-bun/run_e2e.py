#!/usr/bin/env python3
"""Agent trace case for real opencode LLM exchange capture."""

from __future__ import annotations

import argparse
import os
import shutil
import sys
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "runtime_tls"))
from fast_plan import resolve_fast_probe_plan  # noqa: E402
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
    require_llm_exchange_graph,
    require_complete_payload_rows_any,
    require_web_action_tree_projection,
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
    provider: str
    detail: str


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
    actrailweb = require_binary(bin_dir, "actrailweb")
    tls_probe_point_finder = require_binary(bin_dir, "tls-probe-point-finder")
    opencode_entry = require_opencode_entry()
    tls_runtime = resolve_optional_opencode_tls_runtime(
        opencode_entry,
        workload,
        tls_probe_point_finder,
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
        require_llm_exchange_graph(actions)
        web_tree = require_web_action_tree_projection(
            actrailweb,
            resolved_config,
            trace_id,
            float(required(workload, "daemon_ready_timeout_seconds")),
            float(required(workload, "drain_sleep_seconds")),
            required_reachable_kinds=("llm.call", "llm.request", "llm.response", "http.message"),
        )
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
        print(f"opencode_web_action_tree_reachable={web_tree['reachable_count']}")
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
    workload: dict[str, str],
    tls_probe_point_finder: Path,
) -> OpencodeTlsRuntime | None:
    try:
        plan = resolve_fast_probe_plan(
            entry,
            tls_probe_point_finder,
            required(workload, "tls_probe_provider"),
            required(workload, "tls_probe_source"),
            required(workload, "tls_probe_match_limit"),
        )
    except Exception as error:
        print(f"opencode_tls_runtime=disabled {error}")
        return None
    print(f"opencode_tls_runtime=auto {plan.detail}")
    return OpencodeTlsRuntime(provider=plan.provider, detail=plan.detail)


def opencode_config_replacements(tls_runtime: OpencodeTlsRuntime | None) -> dict[str, str]:
    if tls_runtime is None:
        return {
            "__OPENCODE_TLS_ENABLED__": "false",
            "__OPENCODE_SECCOMP_NOTIFY_ENABLED__": "true",
            "__OPENCODE_TLS_REQUIRED_CAPABILITY__": "# tls-plaintext-payload disabled",
        }
    return {
        "__OPENCODE_TLS_ENABLED__": "true",
        "__OPENCODE_SECCOMP_NOTIFY_ENABLED__": "true",
        "__OPENCODE_TLS_REQUIRED_CAPABILITY__": "required_capability = tls-plaintext-payload",
    }


def accepted_payload_sources(tls_runtime: OpencodeTlsRuntime | None) -> list[tuple[str, str]]:
    sources = [("Syscall", "socket-syscall")]
    if tls_runtime is not None:
        sources.insert(0, ("TlsUserSpace", tls_runtime.provider))
    return sources


def accepted_payload_fragments(tls_runtime: OpencodeTlsRuntime | None) -> list[list[str]]:
    fragments = [
        [source, library, "outbound", "Complete", "success"]
        for source, library in accepted_payload_sources(tls_runtime)
    ]
    if tls_runtime is not None:
        fragments.insert(
            0,
            [
                "TlsUserSpace",
                tls_runtime.provider,
                "outbound",
                "inbound",
                "Complete",
                "success",
            ],
        )
    return fragments


def require_tls_response_payloads(payloads: str, tls_runtime: OpencodeTlsRuntime | None) -> int:
    if tls_runtime is None:
        return 0
    return require_complete_payload_rows_any(
        payloads,
        [("TlsUserSpace", tls_runtime.provider)],
        direction="inbound",
    )


def require_executable(path: Path) -> Path:
    if not path.exists() or not os.access(path, os.X_OK):
        raise RuntimeError(f"not an executable: {path}")
    return path.resolve()

if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"opencode agent trace e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
