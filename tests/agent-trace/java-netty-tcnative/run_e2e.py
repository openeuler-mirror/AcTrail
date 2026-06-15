#!/usr/bin/env python3
"""Agent trace case for Java Netty-tcnative native TLS capture."""

from __future__ import annotations

import argparse
import os
import shutil
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
    require_llm_exchange_graph,
    require_otel_span,
    require_root,
    require_web_action_tree_projection,
    required,
    run_checked,
    start_daemon,
    stop_process,
    wait_for_llm_exchange_actions,
    wait_for_payloads_any,
)


JAVA_MAIN_CLASS = "org.actrail.netty.NativeNettyLlmClient"


def main() -> int:
    args = parse_args()
    require_root()
    java = require_tool("java")
    mvn = require_tool("mvn")
    repo = repo_root()
    case_dir = Path(__file__).resolve().parent
    workload = read_config(Path(args.workload_config))
    api_key_env = required(workload, "api_key_env")
    if not os.environ.get(api_key_env):
        raise RuntimeError(f"missing environment variable {api_key_env}")

    prepare_maven_project(
        mvn,
        case_dir,
        float(required(workload, "maven_build_timeout_seconds")),
    )

    bin_dir = repo / args.bin_dir
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
    actrailweb = require_binary(bin_dir, "actrailweb")
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
            "agent-java-netty-tcnative",
            java_argv(java, case_dir, workload),
            float(required(workload, "launch_timeout_seconds")),
        )
        if required(workload, "expected_output_fragment") not in output:
            raise RuntimeError("Netty workload output did not contain expected marker")
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
        require_llm_exchange_graph(actions)
        require_web_action_tree_projection(
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
        print(f"java_netty_tcnative_trace_id={trace_id}")
        print(f"java_netty_tcnative_payload_segments={payload_count}")
        print(f"java_netty_tcnative_llm_request_spans={request_span_count}")
        print(f"java_netty_tcnative_llm_response_spans={response_span_count}")
        print("Java Netty-tcnative native TLS agent trace e2e complete")
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


def require_tool(name: str) -> str:
    path = shutil.which(name)
    if path is None:
        raise RuntimeError(f"missing required tool: {name}")
    return path


def prepare_maven_project(mvn: str, project_dir: Path, timeout_seconds: float) -> None:
    run_checked(
        [mvn, "-q", "-DskipTests", "package"],
        cwd=project_dir,
        timeout=timeout_seconds,
    )


def java_argv(java: str, project_dir: Path, workload: dict[str, str]) -> list[str]:
    classpath = f"{project_dir / 'target/classes'}:{project_dir / 'target/dependency/*'}"
    return [
        java,
        "-cp",
        classpath,
        JAVA_MAIN_CLASS,
        "--api-url",
        required(workload, "api_url"),
        "--api-key-env",
        required(workload, "api_key_env"),
        "--model",
        required(workload, "model"),
        "--prompt",
        required(workload, "prompt"),
        "--aggregation-max-bytes",
        required(workload, "aggregation_max_bytes"),
        "--connect-timeout-ms",
        required(workload, "connect_timeout_ms"),
        "--event-loop-threads",
        required(workload, "event_loop_threads"),
        "--default-https-port",
        required(workload, "default_https_port"),
        "--http-error-status-floor",
        required(workload, "http_error_status_floor"),
    ]


def accepted_payload_sources() -> list[tuple[str, str]]:
    return [("TlsUserSpace", "boringssl")]


def accepted_payload_fragments() -> list[list[str]]:
    return [
        [source, library, "outbound", "Complete", "success"]
        for source, library in accepted_payload_sources()
    ]


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"Java Netty-tcnative agent trace e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
