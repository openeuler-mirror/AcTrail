#!/usr/bin/env python3
"""Agent trace case for a native AgentScope OpenAI-compatible agent."""

from __future__ import annotations

import argparse
import json
import os
import select
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import (  # noqa: E402
    clean_configured_paths,
    emit_llm_otel_evidence,
    export_otel,
    launch_and_parse_trace,
    otel_attrs,
    otel_spans,
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


def main() -> int:
    args = parse_args()
    require_root()
    require_python_package(args.python, "agentscope")
    require_python_package(args.python, "openai")
    repo = repo_root()
    case_dir = Path(__file__).resolve().parent
    workload = read_config(resolve_path(args.workload_config, repo))
    if proxy_mode(workload) == "forward":
        require_env(required(workload, "upstream_api_key_env"))
    bin_dir = resolve_path(args.bin_dir, repo)
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
    actrailweb = require_binary(bin_dir, "actrailweb")
    config = resolve_path(args.config, repo)
    runtime_dir = resolve_path(required(workload, "runtime_dir"), repo)
    runtime_dir.mkdir(parents=True, exist_ok=True)

    proxy = None
    daemon = None
    old_local_key = os.environ.get(required(workload, "local_api_key_env"))
    os.environ[required(workload, "local_api_key_env")] = required(
        workload,
        "local_api_key_value",
    )
    try:
        proxy, proxy_base_url = start_proxy(workload, repo)
        api_url = f"{proxy_base_url.rstrip('/')}/v1"
        clean_configured_paths(actrailctl, config)
        daemon = start_daemon(
            actraild,
            config,
            float(required(workload, "daemon_ready_timeout_seconds")),
        )
        trace_id, output = launch_and_parse_trace(
            actrailctl,
            config,
            "agent-agentscope-openai",
            [
                args.python,
                str(case_dir / "workload.py"),
                "--prompt",
                required(workload, "prompt"),
                "--model",
                required(workload, "model"),
                "--api-url",
                api_url,
                "--api-key-env",
                required(workload, "local_api_key_env"),
                "--request-timeout-seconds",
                required(workload, "request_timeout_seconds"),
                "--max-tokens",
                required(workload, "max_tokens"),
                "--context-size",
                required(workload, "context_size"),
            ],
            float(required(workload, "launch_timeout_seconds")),
        )
        if required(workload, "expected_output_fragment") not in output:
            raise RuntimeError("AgentScope output did not contain expected marker")
        payloads = wait_for_payloads_any(
            actrailctl,
            actrailviewer,
            config,
            trace_id,
            int(required(workload, "drain_attempts")),
            float(required(workload, "drain_sleep_seconds")),
            required(workload, "payload_head"),
            [["Syscall", "socket-syscall", "outbound", "inbound", "Complete", "success"]],
        )
        require_no_tls_payload_rows(payloads)
        outbound_count = require_complete_payload_rows_any(
            payloads,
            [("Syscall", "socket-syscall")],
            direction="outbound",
        )
        inbound_count = require_complete_payload_rows_any(
            payloads,
            [("Syscall", "socket-syscall")],
            direction="inbound",
        )
        actions = wait_for_llm_exchange_actions(
            actrailviewer,
            config,
            trace_id,
            int(required(workload, "drain_attempts")),
            float(required(workload, "drain_sleep_seconds")),
        )
        require_complete_llm_exchange(actions)
        require_no_failed_llm_responses(actions)
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
            resolve_path(required(workload, "otel_output_path"), repo),
        )
        request_span_count = require_otel_span(otel, "llm.request")
        response_span_count = require_otel_span(otel, "llm.response")
        require_plain_http_otel_exchange(otel, workload)
        emit_llm_otel_evidence(otel, int(required(workload, "evidence_text_max_chars")))
        print(f"agentscope_trace_id={trace_id}")
        print(f"agentscope_proxy_base_url={proxy_base_url}")
        print(f"agentscope_api_url={api_url}")
        print(f"agentscope_outbound_payload_segments={outbound_count}")
        print(f"agentscope_inbound_payload_segments={inbound_count}")
        print(f"agentscope_web_action_tree_reachable={web_tree['reachable_count']}")
        print(f"agentscope_llm_request_spans={request_span_count}")
        print(f"agentscope_llm_response_spans={response_span_count}")
        print("AgentScope OpenAI-compatible agent trace e2e complete")
    finally:
        if daemon is not None:
            stop_process(daemon, float(required(workload, "daemon_stop_timeout_seconds")))
        if proxy is not None:
            stop_process(proxy, float(required(workload, "proxy_stop_timeout_seconds")))
            print_process_output("proxy", proxy)
        restore_env(required(workload, "local_api_key_env"), old_local_key)
    return 0


def parse_args() -> argparse.Namespace:
    case_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--config", default=str(case_dir / "operator.conf"))
    parser.add_argument("--workload-config", default=str(case_dir / "workload.conf"))
    parser.add_argument("--python", default=os.environ.get("AGENTSCOPE_PYTHON", sys.executable))
    return parser.parse_args()


def start_proxy(
    workload: dict[str, str],
    repo: Path,
) -> tuple[subprocess.Popen[str], str]:
    proxy_script = repo / "tests/support/llm-http-proxy/provider_proxy.py"
    command = [
        sys.executable,
        str(proxy_script),
        "--mode",
        proxy_mode(workload),
        "--bind-host",
        required(workload, "proxy_bind_host"),
        "--bind-port",
        required(workload, "proxy_bind_port"),
        "--upstream-base-url",
        required(workload, "upstream_base_url"),
        "--upstream-api-key-env",
        required(workload, "upstream_api_key_env"),
        "--upstream-auth-header-name",
        required(workload, "upstream_auth_header_name"),
        "--upstream-auth-scheme",
        required(workload, "upstream_auth_scheme"),
        "--timeout-seconds",
        required(workload, "proxy_request_timeout_seconds"),
        "--read-chunk-bytes",
        required(workload, "proxy_read_chunk_bytes"),
        "--response-chunk-delay-seconds",
        required(workload, "proxy_response_chunk_delay_seconds"),
        "--local-stream-response-text",
        required(workload, "local_stream_response_text"),
    ]
    process = subprocess.Popen(
        command,
        cwd=repo,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    try:
        return process, wait_for_proxy_base_url(
            process,
            float(required(workload, "proxy_ready_timeout_seconds")),
        )
    except Exception:
        stop_process(process, float(required(workload, "proxy_stop_timeout_seconds")))
        print_process_output("proxy", process)
        raise


def wait_for_proxy_base_url(process: subprocess.Popen[str], timeout_seconds: float) -> str:
    deadline = time.monotonic() + timeout_seconds
    if process.stdout is None:
        raise RuntimeError("proxy stdout is not captured")
    while time.monotonic() < deadline:
        line = read_proxy_line(process, deadline)
        if line:
            print(line, end="", flush=True)
            if line.startswith("proxy_base_url="):
                return line.split("=", 1)[1].strip()
        if process.poll() is not None:
            raise RuntimeError("proxy exited before reporting proxy_base_url")
    raise RuntimeError("proxy did not report proxy_base_url before timeout")


def read_proxy_line(process: subprocess.Popen[str], deadline: float) -> str:
    if process.stdout is None:
        raise RuntimeError("proxy stdout is not captured")
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        return ""
    readable, _, _ = select.select([process.stdout], [], [], remaining)
    if readable:
        return process.stdout.readline()
    return ""


def require_plain_http_otel_exchange(document: dict, workload: dict[str, str]) -> None:
    request = matching_otel_action(
        document,
        "llm.request",
        "Syscall",
        required(workload, "model"),
        required(workload, "expected_path_fragment"),
    )
    if request is None:
        raise RuntimeError("OTEL export did not contain a Syscall/plain-HTTP llm.request span")
    response = matching_otel_action(document, "llm.response", "Syscall", "", "")
    if response is None:
        raise RuntimeError("OTEL export did not contain a Syscall/plain-HTTP llm.response span")


def require_no_failed_llm_responses(actions: str) -> None:
    document = json.loads(actions)
    failed = [
        action
        for action in document.get("actions", [])
        if action.get("kind") == "llm.response" and action.get("status") != "success"
    ]
    if failed:
        detail = ", ".join(
            f"{action.get('action_id')}:{action.get('status')}" for action in failed[:5]
        )
        raise RuntimeError(f"actions contained failed llm.response rows: {detail}")


def matching_otel_action(
    document: dict,
    kind: str,
    source_boundary: str,
    model: str,
    path_fragment: str,
) -> dict | None:
    for span in otel_spans(document):
        attrs = otel_attrs(span)
        if attrs.get("actrail.action.kind") != kind:
            continue
        if attrs.get("payload.source_boundary") != source_boundary:
            continue
        if kind == "llm.request" and attrs.get("url.scheme") != "http":
            continue
        if model and attrs.get("llm.request.model") != model:
            continue
        if path_fragment and path_fragment not in attrs.get("url.path", ""):
            continue
        return span
    return None


def require_no_tls_payload_rows(payloads: str) -> None:
    if "TlsUserSpace" in payloads:
        raise RuntimeError("plain HTTP AgentScope E2E unexpectedly produced TlsUserSpace rows")


def proxy_mode(workload: dict[str, str]) -> str:
    return workload.get("proxy_mode", "forward")


def require_python_package(python: str, package: str) -> None:
    command = [
        python,
        "-c",
        "import importlib.util, sys; sys.exit(0 if importlib.util.find_spec(sys.argv[1]) else 1)",
        package,
    ]
    run_checked(command, echo=False)


def require_env(name: str) -> None:
    if not os.environ.get(name):
        raise RuntimeError(f"missing environment variable {name}")


def restore_env(name: str, value: str | None) -> None:
    if value is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = value


def print_process_output(label: str, process: subprocess.Popen[str]) -> None:
    stdout = process.stdout.read() if process.stdout else ""
    stderr = process.stderr.read() if process.stderr else ""
    if stdout:
        print(f"{label}_stdout:\n{stdout}", end="", flush=True)
    if stderr:
        print(f"{label}_stderr:\n{stderr}", end="", file=sys.stderr, flush=True)


def resolve_path(raw: str, repo: Path) -> Path:
    path = Path(raw)
    return path if path.is_absolute() else repo / path


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"AgentScope agent trace e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
