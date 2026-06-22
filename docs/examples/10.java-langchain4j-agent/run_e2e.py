#!/usr/bin/env python3
"""Run the docs LangChain4j OpenAI-compatible agent E2E."""

from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "_common"))
import java_langchain4j as java_workload  # noqa: E402

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


def main() -> int:
    args = parse_args()
    require_root()
    java = java_workload.require_tool("java")
    javac = java_workload.require_tool("javac")
    mvn = java_workload.require_tool("mvn")
    java_workload.require_java_major([java, "-version"], "java")
    java_workload.require_java_major([javac, "-version"], "javac")
    java_workload.require_java_major([mvn, "--version"], "Maven Java runtime")

    repo = repo_root()
    example_dir = Path(__file__).resolve().parent
    project_dir = repo / "docs/examples/_workloads/java-langchain4j-agent"
    settings = read_config(resolve_path(args.workload_config, repo))
    llm = java_workload.resolve_llm_settings(settings, required)
    java_workload.require_https_provider(llm.base_url)
    if not os.environ.get(llm.api_key_env):
        raise RuntimeError(f"missing environment variable {llm.api_key_env}")

    java_workload.prepare_maven_project(
        mvn,
        project_dir,
        float(required(settings, "maven_build_timeout_seconds")),
        run_checked,
    )
    fat_jar = java_workload.require_fat_jar(project_dir)

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
            "java-langchain4j-agent",
            java_workload.java_argv(java, fat_jar, llm),
            float(required(settings, "launch_timeout_seconds")),
        )
        java_workload.require_workload_answer(output, llm, "ACTRAIL_LANGCHAIN4J_AGENT_COMPLETE")
        try:
            payloads = wait_for_payloads_any(
                actrailctl,
                actrailviewer,
                config,
                trace_id,
                int(required(settings, "drain_attempts")),
                float(required(settings, "drain_sleep_seconds")),
                required(settings, "payload_head"),
                accepted_payload_fragments(),
            )
            payload_count = require_complete_payload_rows_any(
                payloads,
                accepted_payload_sources(),
                direction="outbound",
            )
            actions = wait_for_actions(
                actrailviewer,
                config,
                trace_id,
                int(required(settings, "drain_attempts")),
                float(required(settings, "drain_sleep_seconds")),
            )
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
        except RuntimeError as error:
            raise RuntimeError(
                "LangChain4j reached the real LLM, but AcTrail did not capture and "
                "project a complete llm.request. A likely current cause is that Java "
                "JSSE HTTPS plaintext is not visible to the configured TLS plaintext "
                f"capture path. Underlying failure: {error}"
            ) from error
        emit_llm_otel_evidence(otel, int(required(settings, "evidence_text_max_chars")))
        print(f"java_langchain4j_trace_id={trace_id}")
        print(f"java_langchain4j_payload_segments={payload_count}")
        print(f"java_langchain4j_llm_request_spans={request_span_count}")
        print(f"java_langchain4j_llm_response_spans={response_span_count}")
        print(f"java_langchain4j_otel={required(settings, 'otel_output_path')}")
        print("Java LangChain4j OpenAI-compatible agent docs e2e complete")
    finally:
        stop_process(daemon, float(required(settings, "daemon_stop_timeout_seconds")))
    return 0


def parse_args() -> argparse.Namespace:
    example_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--config", default=str(example_dir / "operator.conf"))
    parser.add_argument("--workload-config", default=str(example_dir / "workload.conf"))
    return parser.parse_args()


def accepted_payload_sources() -> list[tuple[str, str]]:
    return [
        ("TlsUserSpace", "jsse"),
    ]


def accepted_payload_fragments() -> list[list[str]]:
    return [
        [source, library, "outbound", "Complete", "success"]
        for source, library in accepted_payload_sources()
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
        if (
            attrs.get("llm.request.model") == model
            and attrs.get("llm.request.payload_bytes")
            and attrs.get("llm.request.content_state") == "canonical_blocks"
            and attrs.get("llm.request.canonical_body_hash")
            and attrs.get("llm.request.block_count")
            and prompt in attrs.get("llm.request.message_preview", "")
            and not attrs.get("llm.request.body_json")
            and not attrs.get("llm.request.body_text")
            and not attrs.get("http.request.body_text")
            and not attrs.get("http.request.body_json")
        ):
            return
    raise RuntimeError("OTEL export did not contain a complete llm.request span with model and payload metadata")


def count_otel_spans(document: dict, kind: str) -> int:
    return sum(1 for span in otel_spans(document) if otel_attrs(span).get("actrail.action.kind") == kind)


def resolve_path(raw: str, repo: Path) -> Path:
    path = Path(raw)
    return path if path.is_absolute() else repo / path


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"Java LangChain4j agent docs e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
