#!/usr/bin/env python3
"""Run a real Claude Code LLM payload E2E with actrailctl launch."""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
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
    required,
    run_checked,
    start_daemon,
    stop_process,
)
from runtime import ClaudeTlsRuntime, resolve_claude_tls_runtime


PayloadSource = tuple[str, str]

TLS_ENABLED_PLACEHOLDER = "__CLAUDE_TLS_ENABLED__"
TLS_BINARY_PLACEHOLDER = "__CLAUDE_TLS_BINARY__"
TLS_RESOLVER_PLACEHOLDER = "__CLAUDE_TLS_RESOLVER__"
TLS_LIBRARY_PLACEHOLDER = "__CLAUDE_TLS_LIBRARY__"
TLS_PATTERN_PLACEHOLDER = "__CLAUDE_TLS_PATTERN_PATH__"
SECCOMP_NOTIFY_PLACEHOLDER = "__CLAUDE_SECCOMP_NOTIFY_ENABLED__"
TLS_REQUIRED_CAPABILITY_PLACEHOLDER = "__CLAUDE_TLS_REQUIRED_CAPABILITY__"


def main() -> int:
    args = parse_args()
    require_root()
    workload_config = read_config(Path(args.workload_config))
    resolved_config = Path(required(workload_config, "resolved_config_path"))
    bin_dir = Path.cwd() / args.bin_dir
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
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
        require_llm_action(actions)
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


def resolve_optional_claude_tls_runtime(values: dict[str, str]) -> ClaudeTlsRuntime | None:
    try:
        runtime = resolve_claude_tls_runtime(values)
    except Exception as error:
        if os.environ.get("CLAUDE_TLS_BINARY"):
            raise
        print(f"claude_tls_runtime=disabled {error}")
        return None
    print(f"claude_tls_runtime={runtime.detail}")
    return runtime


def accepted_payload_sources(
    tls_runtime: ClaudeTlsRuntime | None,
) -> list[PayloadSource]:
    sources = [("Syscall", "socket-syscall")]
    if tls_runtime is not None:
        sources.insert(0, ("TlsUserSpace", tls_runtime.library))
    return sources


def accepted_tls_payload_sources(
    tls_runtime: ClaudeTlsRuntime | None,
) -> list[PayloadSource]:
    if tls_runtime is None:
        return []
    return [("TlsUserSpace", tls_runtime.library)]


def write_resolved_operator_config(
    template_path: Path,
    output_path: Path,
    tls_runtime: ClaudeTlsRuntime | None,
) -> None:
    raw = template_path.read_text(encoding="utf-8")
    if tls_runtime is None:
        replacements = {
            TLS_ENABLED_PLACEHOLDER: "false",
            TLS_BINARY_PLACEHOLDER: "disabled",
            TLS_RESOLVER_PLACEHOLDER: "openssl-symbols",
            TLS_LIBRARY_PLACEHOLDER: "openssl",
            TLS_PATTERN_PLACEHOLDER: "disabled",
            SECCOMP_NOTIFY_PLACEHOLDER: "true",
            TLS_REQUIRED_CAPABILITY_PLACEHOLDER: "# tls-plaintext-payload disabled",
        }
    else:
        replacements = {
            TLS_ENABLED_PLACEHOLDER: "true",
            TLS_BINARY_PLACEHOLDER: str(tls_runtime.binary),
            TLS_RESOLVER_PLACEHOLDER: tls_runtime.resolver,
            TLS_LIBRARY_PLACEHOLDER: tls_runtime.library,
            TLS_PATTERN_PLACEHOLDER: tls_runtime.pattern_path,
            SECCOMP_NOTIFY_PLACEHOLDER: "true",
            TLS_REQUIRED_CAPABILITY_PLACEHOLDER: "required_capability = tls-plaintext-payload",
        }
    for placeholder, value in replacements.items():
        if placeholder not in raw:
            if placeholder == TLS_REQUIRED_CAPABILITY_PLACEHOLDER:
                continue
            raise RuntimeError(f"{template_path} does not contain {placeholder}")
        raw = raw.replace(placeholder, value)
    output_path.write_text(raw, encoding="utf-8")


def wait_for_llm_payloads(
    actrailctl: Path,
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    attempts: int,
    sleep_sec: float,
    head: str,
    accepted_sources: list[PayloadSource],
    accepted_tls_sources: list[PayloadSource],
) -> str:
    for _ in range(attempts):
        run_checked([str(actrailctl), "--config", str(config), "list-traces"], echo=False)
        payloads = run_checked(
            [
                str(actrailviewer),
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
        if payloads_have_required_llm_rows(payloads, accepted_sources, accepted_tls_sources):
            print(payloads, end="")
            return payloads
        time.sleep(sleep_sec)
    detail = ", ".join(f"{source}/{library}" for source, library in accepted_sources)
    raise RuntimeError(f"viewer did not show Claude Code LLM request/response payload rows for {detail}")


def payloads_contain_complete_source(
    payloads: str,
    accepted_sources: list[PayloadSource],
    direction: str,
) -> bool:
    return bool(complete_payload_sources(payloads, accepted_sources, direction))


def payloads_have_required_llm_rows(
    payloads: str,
    accepted_sources: list[PayloadSource],
    accepted_tls_sources: list[PayloadSource],
) -> bool:
    if not payloads_contain_complete_source(payloads, accepted_sources, "outbound"):
        return False
    return tls_response_requirement_satisfied(payloads, accepted_tls_sources)


def tls_response_requirement_satisfied(
    payloads: str,
    accepted_tls_sources: list[PayloadSource],
) -> bool:
    if not accepted_tls_sources:
        return True
    if not payloads_contain_complete_source(payloads, accepted_tls_sources, "outbound"):
        return True
    return payloads_contain_complete_source(payloads, accepted_tls_sources, "inbound")


def complete_payload_sources(
    payloads: str,
    accepted_sources: list[PayloadSource],
    direction: str,
) -> list[PayloadSource]:
    matched: list[PayloadSource] = []
    for line in payloads.splitlines():
        if not re.match(r"^\s*payload-\d+\s+", line):
            continue
        if direction not in line or "Complete" not in line or "success" not in line:
            continue
        for source in accepted_sources:
            boundary, library = source
            if source not in matched and boundary in line and library in line:
                matched.append(source)
    return matched


def require_tls_response_payloads(payloads: str, tls_runtime: ClaudeTlsRuntime | None) -> int:
    return required_tls_response_payload_count(payloads, accepted_tls_payload_sources(tls_runtime))


def required_tls_response_payload_count(
    payloads: str,
    accepted_tls_sources: list[PayloadSource],
) -> int:
    if not accepted_tls_sources:
        return 0
    if not payloads_contain_complete_source(payloads, accepted_tls_sources, "outbound"):
        return 0
    return require_complete_payload_rows_any(payloads, accepted_tls_sources, direction="inbound")


def tls_response_evidence_facts(
    payloads: str,
    accepted_tls_sources: list[PayloadSource],
    response_count: int,
) -> list[str]:
    outbound_tls = complete_payload_sources(payloads, accepted_tls_sources, "outbound")
    inbound_tls = complete_payload_sources(payloads, accepted_tls_sources, "inbound")
    return [
        f"accepted_tls_sources={format_payload_sources(accepted_tls_sources)}",
        f"outbound_tls_sources={format_payload_sources(outbound_tls)}",
        f"inbound_tls_sources={format_payload_sources(inbound_tls)}",
        f"tls_response_required={bool(outbound_tls)}",
        f"captured_response_payload_segments={response_count}",
    ]


def format_payload_sources(sources: list[PayloadSource]) -> str:
    if not sources:
        return "none"
    return ", ".join(f"{source}/{library}" for source, library in sources)


def payload_source_selection_selftest() -> list[str]:
    accepted_tls_sources = [("TlsUserSpace", "boringssl")]
    accepted_sources = [*accepted_tls_sources, ("Syscall", "socket-syscall")]
    http_payloads = (
        "payload-1 trace-1 Syscall socket-syscall outbound Complete success\n"
    )
    https_payloads = (
        "payload-1 trace-1 TlsUserSpace boringssl outbound Complete success\n"
        "payload-2 trace-1 TlsUserSpace boringssl inbound Complete success\n"
    )
    missing_tls_response = (
        "payload-1 trace-1 TlsUserSpace boringssl outbound Complete success\n"
    )
    if not payloads_have_required_llm_rows(http_payloads, accepted_sources, accepted_tls_sources):
        raise RuntimeError("plain HTTP socket payload was treated as requiring a TLS response")
    if required_tls_response_payload_count(http_payloads, accepted_tls_sources) != 0:
        raise RuntimeError("plain HTTP socket payload unexpectedly counted a TLS response")
    if not payloads_have_required_llm_rows(https_payloads, accepted_sources, accepted_tls_sources):
        raise RuntimeError("TLS payload with inbound response did not satisfy TLS response gate")
    if required_tls_response_payload_count(https_payloads, accepted_tls_sources) != 1:
        raise RuntimeError("TLS payload response count was not one")
    if payloads_have_required_llm_rows(missing_tls_response, accepted_sources, accepted_tls_sources):
        raise RuntimeError("TLS outbound payload without inbound TLS response passed the gate")
    return [
        "plain_http_outbound_source=Syscall/socket-syscall",
        "plain_http_tls_response_required=false",
        "https_outbound_source=TlsUserSpace/boringssl",
        "https_tls_response_required=true",
        "https_missing_inbound_tls_response_passes=false",
    ]


def payload_texts(
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    payloads: str,
    fetch_count: int,
) -> str:
    texts: list[str] = []
    for segment_id in parse_segment_ids(payloads)[:fetch_count]:
        texts.append(
            run_checked(
                [
                    str(actrailviewer),
                    "payload",
                    "--config",
                    str(config),
                    "--trace-id",
                    str(trace_id),
                    "--segment-id",
                    segment_id,
                    "--format",
                    "text",
                ],
                echo=False,
            )
        )
    if not texts:
        raise RuntimeError("payloads output did not contain segment ids")
    return "\n".join(texts)


def export_trace_json(
    actrailviewer: Path,
    values: dict[str, str],
    config: Path,
    trace_id: int,
) -> dict:
    output_path = Path(required(values, "export_directory")) / f"trace-{trace_id}-payload.json"
    run_checked(
        [
            str(actrailviewer),
            "export-json",
            "--config",
            str(config),
            "--trace-id",
            str(trace_id),
            "--output",
            str(output_path),
        ],
        echo=False,
    )
    with output_path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def export_trace_otel(
    actrailviewer: Path,
    values: dict[str, str],
    config: Path,
    trace_id: int,
) -> dict:
    output_path = Path(required(values, "export_directory")) / f"trace-{trace_id}-semantic.otlp.json"
    run_checked(
        [
            str(actrailviewer),
            "export-otel",
            "--config",
            str(config),
            "--trace-id",
            str(trace_id),
            "--output",
            str(output_path),
        ],
        echo=False,
    )
    with output_path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def require_exported_payload_marker(document: dict, marker: str) -> None:
    for node in payload_nodes(document):
        attributes = node.get("attributes", {})
        if (
            attributes.get("direction") == "Outbound"
            and attributes.get("bytes_base64")
            and marker in attributes.get("text", "")
        ):
            return
    raise RuntimeError(f"exported JSON payload nodes did not contain prompt marker {marker}")


def wait_for_semantic_actions(
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    attempts: int,
    sleep_sec: float,
) -> str:
    for _ in range(attempts):
        actions = run_checked(
            [
                str(actrailviewer),
                "actions",
                "--config",
                str(config),
                "--trace-id",
                str(trace_id),
            ],
            echo=False,
        )
        if "llm.request" in actions:
            print(actions, end="")
            return actions
        time.sleep(sleep_sec)
    raise RuntimeError("viewer did not show a semantic llm.request action")


def require_llm_action(actions: str) -> None:
    for line in actions.splitlines():
        if "llm.request" in line and "success" in line and "complete" in line:
            return
    raise RuntimeError("semantic actions did not contain a complete successful llm.request")


def require_exported_llm_span(document: dict, marker: str) -> None:
    for span in otel_spans(document):
        attributes = otel_attributes(span)
        body_text = attributes.get("http.request.body_text", "")
        if (
            attributes.get("actrail.action.kind") == "llm.request"
            and attributes.get("actrail.action.completeness") == "complete"
            and attributes.get("actrail.action.status") == "success"
            and attributes.get("llm.request.payload_base64")
            and attributes.get("llm.request.payload_text")
            and attributes.get("llm.request.raw_payload_base64")
            and attributes.get("http.request.body_base64")
            and attributes.get("payload.source_boundary") in {"TlsUserSpace", "Syscall"}
            and attributes.get("url.scheme") in {"http", "https"}
            and body_text.lstrip().startswith("{")
            and '"model"' in body_text
            and marker in attributes.get("llm.request.payload_text", "")
            and (
                attributes.get("http.request.headers_hpack_base64")
                or attributes.get("http.request.headers_text")
            )
        ):
            return
    raise RuntimeError(
        "OTel export did not contain a complete llm.request span with split HTTP headers/body and prompt marker"
    )


def count_action_rows(actions: str) -> int:
    return sum(1 for line in actions.splitlines() if line.startswith("trace:"))


def count_otel_spans(document: dict) -> int:
    return len(otel_spans(document))


def otel_spans(document: dict) -> list[dict]:
    spans: list[dict] = []
    for resource_span in document.get("resourceSpans", []):
        for scope_span in resource_span.get("scopeSpans", []):
            spans.extend(scope_span.get("spans", []))
    return spans


def otel_attributes(span: dict) -> dict[str, str]:
    output: dict[str, str] = {}
    for attribute in span.get("attributes", []):
        value = attribute.get("value", {})
        if "stringValue" in value:
            output[attribute.get("key", "")] = value["stringValue"]
        elif "intValue" in value:
            output[attribute.get("key", "")] = str(value["intValue"])
    return output


def count_payload_nodes(document: dict) -> int:
    return len(payload_nodes(document))


def payload_nodes(document: dict) -> list[dict]:
    return [node for node in document.get("nodes", []) if node.get("kind") == "Payload"]


def parse_segment_ids(payloads: str) -> list[str]:
    ids: list[str] = []
    for line in payloads.splitlines():
        match = re.match(r"^\s*(payload-\d+)\s+", line)
        if match:
            ids.append(match.group(1))
    return ids


def require_non_empty_payload_text(text: str) -> None:
    if not text:
        raise RuntimeError("captured payload text was empty")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"Claude Code LLM payload e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
