#!/usr/bin/env python3
"""Run a real Claude Code LLM payload E2E with actrailctl launch."""

from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "agent-trace"))
from common import (  # noqa: E402
    clean_configured_paths,
    emit_llm_otel_evidence,
    launch_and_parse_trace_with_daemon,
    read_config,
    require_binary,
    require_complete_payload_rows_any,
    require_root,
    require_web_action_tree_projection,
    required,
    start_daemon,
    stop_process,
)
from config_template import (  # noqa: E402
    PayloadSource,
    accepted_payload_sources,
    accepted_tls_payload_sources,
    resolve_optional_claude_tls_runtime,
    write_resolved_operator_config,
)
from export_checks import (  # noqa: E402
    count_action_rows,
    count_otel_spans,
    count_payload_nodes,
    export_trace_json,
    export_trace_otel,
    require_exported_llm_response_span,
    require_exported_llm_span,
    require_exported_payload_marker,
    require_llm_exchange,
    require_llm_exchange_graph,
    wait_for_semantic_actions,
)
from payload_checks import (  # noqa: E402
    payload_source_selection_selftest,
    payload_texts,
    require_non_empty_payload_text,
    require_tls_response_payloads,
    tls_response_evidence_facts,
    wait_for_llm_payloads,
)


def main() -> int:
    args = parse_args()
    require_root()
    workload_config = read_config(Path(args.workload_config))
    resolved_config = Path(required(workload_config, "resolved_config_path"))
    bin_dir = Path.cwd() / args.bin_dir
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
    actrailweb = require_binary(bin_dir, "actrailweb")
    tls_runtime = resolve_optional_claude_tls_runtime(workload_config)
    write_resolved_operator_config(Path(args.config_template), resolved_config, tls_runtime)
    values = read_config(resolved_config)
    clean_configured_paths(actrailctl, resolved_config)
    daemon = start_daemon(
        actraild,
        resolved_config,
        float(required(workload_config, "daemon_ready_timeout_seconds")),
    )
    try:
        trace_id, _launch_output = launch_and_parse_trace_with_daemon(
            daemon,
            actrailctl,
            resolved_config,
            "claude-code-real-e2e",
            ["claude", "-p", required(workload_config, "prompt")],
            float(required(workload_config, "claude_timeout_seconds")),
            float(required(workload_config, "launch_poll_interval_seconds")),
            float(required(workload_config, "launch_stop_timeout_seconds")),
        )
        payloads = wait_for_llm_payloads(
            actrailctl,
            actrailviewer,
            resolved_config,
            trace_id,
            int(required(workload_config, "drain_attempts")),
            float(required(workload_config, "drain_sleep_seconds")),
            required(workload_config, "payload_head"),
            accepted_payload_sources(tls_runtime),
            accepted_tls_payload_sources(tls_runtime),
        )
        payload_count = require_complete_payload_rows_any(
            payloads,
            accepted_payload_sources(tls_runtime),
            direction="outbound",
        )
        response_payload_count = require_tls_response_payloads(payloads, tls_runtime)
        actions = wait_for_semantic_actions(
            actrailviewer,
            resolved_config,
            trace_id,
            int(required(workload_config, "drain_attempts")),
            float(required(workload_config, "drain_sleep_seconds")),
        )
        require_llm_exchange(actions)
        require_llm_exchange_graph(actions)
        web_tree = require_web_action_tree_projection(
            actrailweb,
            resolved_config,
            trace_id,
            float(required(workload_config, "daemon_ready_timeout_seconds")),
            float(required(workload_config, "drain_sleep_seconds")),
            required_reachable_kinds=("llm.call", "llm.request", "llm.response", "http.message"),
        )
        text = payload_texts(
            actrailviewer,
            resolved_config,
            trace_id,
            payloads,
            int(required(workload_config, "payload_fetch_count")),
        )
        require_non_empty_payload_text(text)
        marker = required(workload_config, "prompt_marker")
        export_document = export_trace_json(actrailviewer, values, resolved_config, trace_id)
        require_exported_payload_marker(export_document, marker)
        otel_document = export_trace_otel(actrailviewer, values, resolved_config, trace_id)
        require_exported_llm_span(otel_document, marker)
        require_exported_llm_response_span(otel_document)
        emit_llm_otel_evidence(
            otel_document,
            int(required(workload_config, "evidence_text_max_chars")),
        )
        print(f"claude_code_trace_id={trace_id}")
        print(f"captured_payload_segments={payload_count}")
        print(f"captured_response_payload_segments={response_payload_count}")
        print(f"captured_payload_text_bytes={len(text.encode('utf-8'))}")
        print(f"exported_payload_nodes={count_payload_nodes(export_document)}")
        print(f"exported_payload_marker={marker}")
        print(f"semantic_action_rows={count_action_rows(actions)}")
        print(f"web_action_tree_reachable={web_tree['reachable_count']}")
        print(f"otel_semantic_spans={count_otel_spans(otel_document)}")
        print(f"otel_payload_marker={marker}")
        print("claude code LLM payload e2e complete")
    finally:
        stop_process(daemon, float(required(workload_config, "daemon_stop_timeout_seconds")))
    return 0


def parse_args() -> argparse.Namespace:
    test_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--config-template", default=str(test_dir / "operator.conf"))
    parser.add_argument("--workload-config", default=str(test_dir / "workload.conf"))
    return parser.parse_args()


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"Claude Code LLM payload e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
