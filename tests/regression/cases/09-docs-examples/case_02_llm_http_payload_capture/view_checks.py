"""Payload/Application view checks for docs HTTP payload regression."""

from __future__ import annotations

import os
import time
from pathlib import Path

from model import FAIL, PASS, SKIP
from workload_config import required

from helpers import (
    actrail_command,
    bullet_evidence,
    evidence_detail,
    event_domain_line,
    first_content_line,
    run_command,
    viewer,
)

CURL_HTTP_VERSION_MARKER = "ACTRAIL_CURL_HTTP_VERSION="
CURL_HTTP1_VERSION = "1.1"
CURL_HTTP2_VERSION = "2"


class ViewEvidenceError(RuntimeError):
    def __init__(
        self,
        message: str,
        payloads: str,
        events: str,
        diagnostics: str,
        launch_output: str,
    ) -> None:
        super().__init__(message)
        self.payloads = payloads
        self.events = events
        self.diagnostics = diagnostics
        self.launch_output = launch_output


def wait_payload_application_views(
    env,
    config: Path | None,
    trace_id: int,
    workload: dict[str, str],
    protocol: str,
    launch_output: str,
    require_wide_capture: bool,
    require_stdio_payload: bool,
) -> tuple[str, str, str]:
    last_payloads = ""
    last_events = ""
    last_diagnostics = ""
    last_payload_ok = False
    last_event_ok = False
    last_wide_capture_ok = not require_wide_capture
    last_payload_line = "not checked"
    last_event_line = "not checked"
    last_socket_payload_line = "not checked"
    last_stdio_payload_line = "not checked"
    last_any_application_line = "not checked"
    last_curl_exec_line = "not checked"
    last_diagnostics_line = "not checked"
    last_curl_http_version = curl_http_version_line(launch_output) or "missing ACTRAIL_CURL_HTTP_VERSION marker"
    for _ in range(int(required(workload, "drain_attempts"))):
        drain_trace(env, config, workload)
        payloads = viewer(env, config, "payloads", trace_id)
        events = viewer(env, config, "events", trace_id)
        diagnostics = viewer(env, config, "diagnostics", trace_id)
        last_payloads = payloads
        last_events = events
        last_diagnostics = diagnostics
        payload_line = find_line(payloads, ("TlsUserSpace", "Complete", "success"))
        socket_payload_line = find_line(payloads, ("Syscall",))
        stdio_payload_line = find_line(payloads, ("Stdio",))
        payload_ok = payload_line is not None
        if protocol == "http1.1":
            chat_path = os.environ.get("ACTRAIL_LLM_CHAT_PATH", "/chat/completions")
            event_line = find_event_line(events, "Application", "request", (f"POST {chat_path}",))
        else:
            event_line = find_event_line(events, "Application", "frame", ()) or find_event_line(
                events,
                "Application",
                "data",
                (),
            )
        event_ok = event_line is not None
        wide_capture_ok = not require_wide_capture or (
            socket_payload_line is not None and (not require_stdio_payload or stdio_payload_line is not None)
        )
        last_payload_ok = payload_ok
        last_event_ok = event_ok
        last_wide_capture_ok = wide_capture_ok
        last_payload_line = payload_line or "missing line containing TlsUserSpace, Complete, success"
        last_event_line = event_line or expected_event_missing_detail(protocol)
        last_socket_payload_line = socket_payload_line or "missing Syscall socket payload row"
        last_stdio_payload_line = stdio_payload_line or stdio_missing_detail(require_stdio_payload)
        last_any_application_line = event_domain_line(events, "Application") or "missing any Application row"
        last_curl_exec_line = find_event_line(events, "Process", "exec", ("curl",)) or "missing curl exec row"
        last_diagnostics_line = first_content_line(diagnostics)
        if payload_ok and event_ok and wide_capture_ok:
            return payloads, events, diagnostics
        time.sleep(float(required(workload, "drain_sleep_seconds")))
    raise ViewEvidenceError(
        evidence_detail(
            f"{protocol} documented payload/Application rows",
            bullet_evidence(
                [
                    f"payload_ok={last_payload_ok}",
                    f"event_ok={last_event_ok}",
                    f"wide_capture_ok={last_wide_capture_ok}",
                    f"stdio_required={require_stdio_payload}",
                    f"payload_line={last_payload_line}",
                    f"event_line={last_event_line}",
                    f"socket_payload_line={last_socket_payload_line}",
                    f"stdio_payload_line={last_stdio_payload_line}",
                    f"any_application_line={last_any_application_line}",
                    f"curl_exec_line={last_curl_exec_line}",
                    f"curl_http_version={last_curl_http_version}",
                    f"diagnostics_line={last_diagnostics_line}",
                    f"launch_output_bytes={len(launch_output.encode('utf-8'))}",
                    f"launch_contains_chat_completion={'chat.completion' in launch_output}",
                    f"payload_output_bytes={len(last_payloads.encode('utf-8'))}",
                    f"event_output_bytes={len(last_events.encode('utf-8'))}",
                    f"diagnostics_output_bytes={len(last_diagnostics.encode('utf-8'))}",
                ],
            ),
        ),
        last_payloads,
        last_events,
        last_diagnostics,
        launch_output,
    )


def wide_capture_expected(require_stdio_payload: bool) -> str:
    if require_stdio_payload:
        return "Syscall socket payload rows and configured Stdio stdout rows are present"
    return "Syscall socket payload rows are present; Stdio stdout is not required when storage mode drops it"


def wide_capture_reason(require_stdio_payload: bool) -> str:
    if require_stdio_payload:
        return "provider docs config must collect configured side-channel evidence to debug TLS/Application misses"
    return "provider docs config intentionally drops curl stdout before persistence"


def view_failure_transcript(error: ViewEvidenceError, daemon_output: str) -> str:
    return "\n".join(
        (
            "=== actrailctl launch output ===",
            error.launch_output,
            "=== actrailviewer payloads ===",
            error.payloads,
            "=== actrailviewer events ===",
            error.events,
            "=== actrailviewer diagnostics ===",
            error.diagnostics,
            daemon_output,
        )
    )


def drain_trace(env, config: Path | None, workload: dict[str, str]) -> None:
    run_command(
        env,
        actrail_command(env, "actrailctl", config, "list-traces"),
        float(required(workload, "control_timeout_seconds")),
    )


def find_line(output: str, fragments: tuple[str, ...]) -> str | None:
    for line in output.splitlines():
        if all(fragment in line for fragment in fragments):
            return line
    return None


def find_event_line(
    output: str,
    domain: str,
    operation: str,
    detail_fragments: tuple[str, ...],
) -> str | None:
    for line in output.splitlines():
        columns = line.split()
        if (
            len(columns) >= 4
            and columns[0].startswith("event-")
            and columns[1] == domain
            and columns[3] == operation
            and all(fragment in line for fragment in detail_fragments)
        ):
            return line
    return None


def expected_event_missing_detail(protocol: str) -> str:
    if protocol == "http1.1":
        chat_path = os.environ.get("ACTRAIL_LLM_CHAT_PATH", "/chat/completions")
        return f"missing Application request row containing POST {chat_path}"
    return "missing Application frame/data row"


def stdio_missing_detail(require_stdio_payload: bool) -> str:
    if require_stdio_payload:
        return "missing Stdio payload row"
    return "not required because curl stdout storage policy drops stdout"


def curl_http_version_line(output: str) -> str | None:
    for line in output.splitlines():
        if line.startswith(CURL_HTTP_VERSION_MARKER):
            return line
    return None


def expected_curl_http_version(protocol: str) -> str:
    return CURL_HTTP2_VERSION if protocol == "http2" else CURL_HTTP1_VERSION


def curl_http_version_status(protocol: str, curl_http_version: str | None) -> str:
    expected = f"{CURL_HTTP_VERSION_MARKER}{expected_curl_http_version(protocol)}"
    if curl_http_version == expected:
        return PASS
    if protocol == "http2" and curl_http_version == f"{CURL_HTTP_VERSION_MARKER}{CURL_HTTP1_VERSION}":
        return SKIP
    return FAIL


def expected_curl_http_version_detail(protocol: str, expected_curl_version: str) -> str:
    if protocol == "http2":
        return (
            f"curl reports final HTTP version {expected_curl_version}; "
            "skip when provider/proxy ALPN negotiates HTTP/1.1"
        )
    return f"curl reports final HTTP version {expected_curl_version}"


def curl_http_version_evidence(protocol: str, curl_http_version: str | None) -> str:
    if protocol == "http2" and curl_http_version == f"{CURL_HTTP_VERSION_MARKER}{CURL_HTTP1_VERSION}":
        return (
            "external provider path negotiated HTTP/1.1 instead of HTTP/2; "
            "the external HTTP/1.1 docs case covers this provider path, and the local HTTP/2 docs case "
            "covers AcTrail HTTP/2 payload analysis without provider/proxy ALPN dependency"
        )
    return "provider docs workload must make protocol negotiation explicit"
