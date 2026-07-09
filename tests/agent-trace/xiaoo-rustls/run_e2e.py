#!/usr/bin/env python3
"""Agent trace case for real xiaoO LLM exchange capture."""

from __future__ import annotations

import argparse
import os
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "runtime_tls"))
from common import (  # noqa: E402
    clean_configured_paths,
    emit_llm_otel_evidence,
    export_otel,
    launch_and_parse_trace,
    read_config,
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
from rustls import resolve_rustls_probe_plan  # noqa: E402


def main() -> int:
    args = parse_args()
    require_root()
    repo = repo_root()
    workload = read_config(Path(args.workload_config))
    bin_dir = repo / args.bin_dir
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
    actrailweb = require_binary(bin_dir, "actrailweb")
    tls_probe_point_finder = require_binary(bin_dir, "tls-probe-point-finder")
    xiaoo_binary = resolve_xiaoo_binary(required(workload, "xiaoo_binary"))
    tls_runtime = resolve_optional_xiaoo_tls_runtime(
        xiaoo_binary,
        workload,
        tls_probe_point_finder,
    )
    config = Path(args.config)
    clean_configured_paths(actrailctl, config)
    daemon = start_daemon(
        actraild,
        config,
        float(required(workload, "daemon_ready_timeout_seconds")),
    )
    try:
        trace_id, output = launch_and_parse_trace(
            actrailctl,
            config,
            "agent-xiaoo-rustls",
            [
                str(xiaoo_binary),
                "--cli",
                "run",
                "--no-tools",
                "--max-turns",
                "1",
                "--prompt",
                required(workload, "prompt"),
            ],
            float(required(workload, "launch_timeout_seconds")),
        )
        if required(workload, "expected_output_fragment") not in output:
            raise RuntimeError("xiaoO output did not contain expected marker")
        payloads = wait_for_payloads_any(
            actrailctl,
            actrailviewer,
            config,
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
        actions = wait_for_llm_exchange_actions(
            actrailviewer,
            config,
            trace_id,
            int(required(workload, "drain_attempts")),
            float(required(workload, "drain_sleep_seconds")),
        )
        require_complete_llm_exchange(actions)
        require_llm_exchange_graph(actions)
        web_tree = require_web_action_tree_projection(
            actrailweb,
            config,
            trace_id,
            float(required(workload, "daemon_ready_timeout_seconds")),
            float(required(workload, "drain_sleep_seconds")),
            required_reachable_kinds=("llm.call", "llm.request", "llm.response", "http.message"),
        )
        otel = export_otel(
            actrailviewer,
            config,
            trace_id,
            Path(required(workload, "otel_output_path")),
        )
        request_span_count = require_otel_span(otel, "llm.request")
        response_span_count = require_otel_span(otel, "llm.response")
        emit_llm_otel_evidence(otel, int(required(workload, "evidence_text_max_chars")))
        print(f"xiaoo_trace_id={trace_id}")
        print(f"xiaoo_payload_segments={payload_count}")
        print(f"xiaoo_web_action_tree_reachable={web_tree['reachable_count']}")
        print(f"xiaoo_llm_request_spans={request_span_count}")
        print(f"xiaoo_llm_response_spans={response_span_count}")
        print("xiaoO agent trace e2e complete")
    finally:
        stop_process(daemon, float(required(workload, "daemon_stop_timeout_seconds")))
    return 0


def parse_args() -> argparse.Namespace:
    case_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--config", default=str(case_dir / "operator.patch.conf"))
    parser.add_argument("--workload-config", default=str(case_dir / "workload.conf"))
    return parser.parse_args()


def resolve_xiaoo_binary(configured: str) -> Path:
    raw = os.environ.get("XIAOO_BINARY", configured)
    path = Path(raw)
    if path.parent == Path("."):
        resolved = shutil.which(raw)
        if resolved is None:
            raise RuntimeError(f"xiaoO executable is not on PATH: {raw}")
        return require_executable(Path(resolved))
    return require_executable(path)


def resolve_optional_xiaoo_tls_runtime(
    xiaoo_binary: Path,
    workload: dict[str, str],
    tls_probe_point_finder: Path,
) -> object | None:
    try:
        plan = resolve_rustls_probe_plan(xiaoo_binary, workload, tls_probe_point_finder)
    except Exception as error:
        print(f"xiaoo_rustls_auto_plan=disabled {error}")
        return None
    print(f"xiaoo_rustls_auto_plan={plan.detail}")
    return plan


def accepted_payload_sources(tls_runtime: object | None) -> list[tuple[str, str]]:
    sources = [("Syscall", "socket-syscall")]
    if tls_runtime is not None:
        sources.insert(0, ("TlsUserSpace", "rustls"))
    return sources


def accepted_payload_fragments(tls_runtime: object | None) -> list[list[str]]:
    return [
        [source, library, "outbound", "Complete", "success"]
        for source, library in accepted_payload_sources(tls_runtime)
    ]


def require_executable(path: Path) -> Path:
    if not path.exists() or not os.access(path, os.X_OK):
        raise RuntimeError(f"not an executable: {path}")
    return path.resolve()


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"xiaoO agent trace e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
