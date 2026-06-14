#!/usr/bin/env python3
"""Run the docs LangGraph OpenAI-compatible agent E2E."""

from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tests/agent-trace"))
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
    require_complete_llm_action,
    require_complete_payload_rows_any,
    require_otel_span,
    require_root,
    required,
    run_checked,
    start_daemon,
    stop_process,
    wait_for_actions,
    wait_for_payloads_any,
)


@dataclass(frozen=True)
class LlmSettings:
    prompt: str
    expected_output_fragment: str
    model: str
    base_url: str
    api_key_env: str
    request_timeout_seconds: str
    prompt_overridden: bool
    expected_overridden: bool


def main() -> int:
    args = parse_args()
    require_root()
    require_python_package(args.python, "langgraph")
    require_python_package(args.python, "langchain_openai")

    repo = repo_root()
    example_dir = Path(__file__).resolve().parent
    settings = read_config(resolve_path(args.workload_config, repo))
    llm = resolve_llm_settings(settings)
    if not os.environ.get(llm.api_key_env):
        raise RuntimeError(f"missing environment variable {llm.api_key_env}")

    bin_dir = resolve_path(args.bin_dir, repo)
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
    config = resolve_path(args.config, repo)

    clean_configured_paths(actrailctl, config)
    daemon = start_daemon(
        actraild,
        config,
        float(required(settings, "daemon_ready_timeout_seconds")),
    )
    try:
        trace_id, output = launch_and_parse_trace(
            actrailctl,
            config,
            "python-langgraph-agent",
            [
                args.python,
                str(example_dir / "workload.py"),
                "--prompt",
                llm.prompt,
                "--model",
                llm.model,
                "--base-url",
                llm.base_url,
                "--api-key-env",
                llm.api_key_env,
                "--request-timeout-seconds",
                llm.request_timeout_seconds,
            ],
            float(required(settings, "launch_timeout_seconds")),
        )
        require_workload_answer(output, llm, "ACTRAIL_LANGGRAPH_AGENT_COMPLETE")
        accepted_sources = accepted_payload_sources(llm.base_url)
        try:
            payloads = wait_for_payloads_any(
                actrailctl,
                actrailviewer,
                config,
                trace_id,
                int(required(settings, "drain_attempts")),
                float(required(settings, "drain_sleep_seconds")),
                required(settings, "payload_head"),
                accepted_payload_fragments(accepted_sources),
            )
        except RuntimeError as error:
            raise RuntimeError(
                "LangGraph reached the real LLM through the normal LangChain/OpenAI SDK "
                "path, but AcTrail did not capture an accepted complete outbound request "
                "payload. HTTPS providers require AcTrail to observe the runtime TLS "
                "plaintext path; Syscall/socket-syscall rows for proxy CONNECT tunnels "
                "do not prove chat request body capture. "
                f"Underlying failure: {error}"
            ) from error
        payload_count = require_complete_payload_rows_any(
            payloads,
            accepted_sources,
            direction="outbound",
        )
        try:
            actions = wait_for_actions(
                actrailviewer,
                config,
                trace_id,
                int(required(settings, "drain_attempts")),
                float(required(settings, "drain_sleep_seconds")),
            )
        except RuntimeError as error:
            raise RuntimeError(
                "LangGraph reached the real LLM through the normal LangChain/OpenAI SDK "
                "path and an accepted payload row was visible, but AcTrail did not "
                "project a complete llm.request action. "
                f"Underlying failure: {error}"
            ) from error
        require_complete_llm_action(actions)
        otel = export_otel(
            actrailviewer,
            config,
            trace_id,
            Path(required(settings, "otel_output_path")),
        )
        request_span_count = require_otel_span(otel, "llm.request")
        response_span_count = count_otel_spans(otel, "llm.response")
        require_otel_request_evidence(otel, llm.model, llm.prompt)
        emit_llm_otel_evidence(otel, int(required(settings, "evidence_text_max_chars")))
        print(f"python_langgraph_trace_id={trace_id}")
        print(f"python_langgraph_payload_segments={payload_count}")
        print(f"python_langgraph_llm_request_spans={request_span_count}")
        print(f"python_langgraph_llm_response_spans={response_span_count}")
        print(f"python_langgraph_otel={required(settings, 'otel_output_path')}")
        print("Python LangGraph OpenAI-compatible agent docs e2e complete")
    finally:
        stop_process(daemon, float(required(settings, "daemon_stop_timeout_seconds")))
    return 0


def parse_args() -> argparse.Namespace:
    example_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--config", default=str(example_dir / "operator.conf"))
    parser.add_argument("--workload-config", default=str(example_dir / "workload.conf"))
    parser.add_argument("--python", default=os.environ.get("LANGGRAPH_PYTHON", sys.executable))
    return parser.parse_args()


def resolve_llm_settings(values: dict[str, str]) -> LlmSettings:
    prompt_overridden = "ACTRAIL_LLM_PROMPT" in os.environ
    expected_overridden = "ACTRAIL_LLM_EXPECTED_OUTPUT_FRAGMENT" in os.environ
    prompt = os.environ.get("ACTRAIL_LLM_PROMPT", required(values, "prompt"))
    model = os.environ.get("ACTRAIL_LLM_MODEL", required(values, "model"))
    api_key_env = os.environ.get("ACTRAIL_LLM_API_KEY_ENV", required(values, "api_key_env"))
    expected = os.environ.get(
        "ACTRAIL_LLM_EXPECTED_OUTPUT_FRAGMENT",
        required(values, "expected_output_fragment"),
    )
    return LlmSettings(
        prompt=prompt,
        expected_output_fragment=expected,
        model=model,
        base_url=resolve_langchain_openai_base_url(values),
        api_key_env=api_key_env,
        request_timeout_seconds=required(values, "request_timeout_seconds"),
        prompt_overridden=prompt_overridden,
        expected_overridden=expected_overridden,
    )


def resolve_langchain_openai_base_url(values: dict[str, str]) -> str:
    base_url = os.environ.get("ACTRAIL_LLM_BASE_URL", required(values, "base_url"))
    chat_path = os.environ.get("ACTRAIL_LLM_CHAT_PATH", required(values, "chat_path"))
    endpoint = join_url(base_url, chat_path)
    suffix = "/chat/completions"
    if not endpoint.endswith(suffix):
        raise RuntimeError(
            "ChatOpenAI appends /chat/completions; "
            "ACTRAIL_LLM_CHAT_PATH must end with /chat/completions"
        )
    base = endpoint[: -len(suffix)]
    return base or "/"


def require_python_package(python: str, package: str) -> None:
    command = [
        python,
        "-c",
        "import importlib.util, sys; sys.exit(0 if importlib.util.find_spec(sys.argv[1]) else 1)",
        package,
    ]
    run_checked(command, echo=False)


def require_workload_answer(output: str, llm: LlmSettings, completion_marker: str) -> None:
    if completion_marker not in output:
        raise RuntimeError("workload did not report completion after the LLM call")
    answer = parse_answer(output)
    if not answer.strip():
        raise RuntimeError("workload returned an empty LLM answer")
    if not llm.prompt_overridden or llm.expected_overridden:
        if llm.expected_output_fragment not in answer and llm.expected_output_fragment not in output:
            raise RuntimeError("LLM answer did not contain the expected marker")


def parse_answer(output: str) -> str:
    for line in output.splitlines():
        if line.startswith("llm_answer_json="):
            return str(json.loads(line.split("=", 1)[1]))
    raise RuntimeError("workload output did not include llm_answer_json")


def accepted_payload_sources(base_url: str) -> list[tuple[str, str]]:
    if base_url.lower().startswith("http://"):
        return [("Syscall", "socket-syscall")]
    return [("TlsUserSpace", "openssl")]


def accepted_payload_fragments(accepted: list[tuple[str, str]]) -> list[list[str]]:
    return [
        [source, library, "outbound", "Complete", "success"]
        for source, library in accepted
    ]


def require_otel_request_evidence(document: dict, model: str, prompt: str) -> None:
    for span in otel_spans(document):
        attrs = otel_attrs(span)
        if attrs.get("actrail.action.kind") != "llm.request":
            continue
        if attrs.get("actrail.action.completeness") != "complete":
            continue
        if attrs.get("actrail.action.status") != "success":
            continue
        body = attrs.get("http.request.body_text") or attrs.get("llm.request.payload_text", "")
        if attrs.get("llm.request.model") == model and prompt in body:
            return
    raise RuntimeError("OTEL export did not contain a complete llm.request span with model and prompt")


def count_otel_spans(document: dict, kind: str) -> int:
    return sum(1 for span in otel_spans(document) if otel_attrs(span).get("actrail.action.kind") == kind)


def resolve_path(raw: str, repo: Path) -> Path:
    path = Path(raw)
    return path if path.is_absolute() else repo / path


def join_url(base_url: str, path: str) -> str:
    return base_url.rstrip("/") + "/" + path.lstrip("/")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"Python LangGraph agent docs e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
