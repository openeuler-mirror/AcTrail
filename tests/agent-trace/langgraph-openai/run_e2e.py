#!/usr/bin/env python3
"""Agent trace case for a real LangGraph Python LLM exchange."""

from __future__ import annotations

import argparse
import importlib.util
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import (  # noqa: E402
    clean_configured_paths,
    emit_llm_otel_evidence,
    export_otel,
    launch_and_parse_trace,
    read_config,
    repo_root,
    require_binary,
    require_complete_llm_exchange,
    require_complete_payload_rows_any,
    require_otel_span,
    require_root,
    required,
    run_checked,
    start_daemon,
    stop_process,
    wait_for_llm_exchange_actions,
    wait_for_payloads_any,
)


def main() -> int:
    args = parse_args()
    require_root()
    require_python_package(args.python, "langgraph")
    require_python_package(args.python, "requests")
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
            "agent-langgraph-openai",
            [
                args.python,
                str(case_dir / "workload.py"),
                "--prompt",
                required(workload, "prompt"),
                "--model",
                required(workload, "model"),
                "--api-url",
                required(workload, "api_url"),
                "--api-key-env",
                api_key_env,
            ],
            float(required(workload, "launch_timeout_seconds")),
        )
        if required(workload, "expected_output_fragment") not in output:
            raise RuntimeError("LangGraph output did not contain expected marker")
        payloads = wait_for_payloads_any(
            actrailctl,
            actrailviewer,
            config,
            trace_id,
            int(required(workload, "drain_attempts")),
            float(required(workload, "drain_sleep_seconds")),
            required(workload, "payload_head"),
            accepted_payload_fragments(),
        )
        payload_count = require_complete_payload_rows_any(
            payloads,
            accepted_payload_sources(),
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
        otel = export_otel(
            actrailviewer,
            config,
            trace_id,
            Path(required(workload, "otel_output_path")),
        )
        request_span_count = require_otel_span(otel, "llm.request")
        response_span_count = require_otel_span(otel, "llm.response")
        emit_llm_otel_evidence(otel, int(required(workload, "evidence_text_max_chars")))
        print(f"langgraph_trace_id={trace_id}")
        print(f"langgraph_payload_segments={payload_count}")
        print(f"langgraph_llm_request_spans={request_span_count}")
        print(f"langgraph_llm_response_spans={response_span_count}")
        print("LangGraph OpenAI-compatible agent trace e2e complete")
    finally:
        stop_process(daemon, float(required(workload, "daemon_stop_timeout_seconds")))
    return 0


def parse_args() -> argparse.Namespace:
    case_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--config", default=str(case_dir / "operator.conf"))
    parser.add_argument("--workload-config", default=str(case_dir / "workload.conf"))
    parser.add_argument("--python", default=os.environ.get("LANGGRAPH_PYTHON", sys.executable))
    return parser.parse_args()


def require_python_package(python: str, package: str) -> None:
    command = [
        python,
        "-c",
        "import importlib.util, sys; sys.exit(0 if importlib.util.find_spec(sys.argv[1]) else 1)",
        package,
    ]
    run_checked(command, echo=False)


def accepted_payload_sources() -> list[tuple[str, str]]:
    return [("TlsUserSpace", "openssl"), ("Syscall", "socket-syscall")]


def accepted_payload_fragments() -> list[list[str]]:
    return [
        [source, library, "outbound", "Complete", "success"]
        for source, library in accepted_payload_sources()
    ]


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"LangGraph agent trace e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
