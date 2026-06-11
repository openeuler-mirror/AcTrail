"""Docs example 02 HTTP payload regression steps."""

from __future__ import annotations

import os
import shlex
import shutil
import time
from pathlib import Path

from model import FAIL, PASS, SKIP, CaseResult
from workload_config import required

from helpers import (
    add_expected_found_check,
    actrail_command,
    bullet_evidence,
    clean_default_operator_state,
    evidence_detail,
    evidence_rows,
    event_domain_line,
    event_rows,
    fail_step,
    first_content_line,
    parse_trace_id,
    process_artifact_transcript,
    record_process_artifacts,
    run_clean,
    run_command,
    start_daemon,
    start_process,
    stop_process,
    viewer,
    wait_for_output,
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


def run_http2_local(env, result: CaseResult, workload: dict[str, str]) -> str:
    name = "docs 02 local HTTP/2 payload"
    config = None
    script = env.repo_root / "docs/examples/02.llm-http-payload-capture/http2-local/workload.py"
    target_config = env.repo_root / "docs/examples/02.llm-http-payload-capture/http2-local/workload.conf"
    result.begin_check(name, "running local TLS HTTP/2 workload")
    daemon = None
    server = None
    launch_output = ""
    try:
        run_clean(env, "http2-local", workload)
        clean_default_operator_state(env, workload)
        daemon = start_daemon(env, config, workload)
        record_process_artifacts(result, daemon)
        server = start_process(
            env,
            [
                env.python,
                str(script),
                "--target-config",
                str(target_config),
                "--serve-only",
            ],
        )
        server_port = parse_server_port(
            wait_for_output(
                server,
                "\n",
                float(required(workload, "http2_local_launch_timeout_seconds")),
            )
        )
        launch = run_command(
            env,
            actrail_command(
                env,
                "actrailctl",
                config,
                "launch",
                "--name",
                "docs-http2-local",
                "--",
                "curl",
                "--http2",
                "--silent",
                "--show-error",
                "--insecure",
                "--request",
                "POST",
                "--header",
                "Content-Type: application/json",
                "--data",
                '{"model":"actrail-http2","messages":[{"role":"user","content":"payload capture over h2"}],"stream":false}',
                f"https://127.0.0.1:{server_port}/v1/chat/completions",
            ),
            float(required(workload, "http2_local_launch_timeout_seconds")),
        )
        launch_output = launch.output
        wait_for_server_exit(server, workload)
        server = None
        trace_id = parse_trace_id(launch.output)
        payloads, events, diagnostics = wait_payload_application_views(
            env,
            config,
            trace_id,
            workload,
            "http2",
            launch_output,
            require_wide_capture=False,
        )
        result.stdout_tail = env.output_tail("\n".join((payloads, events, diagnostics)))
        add_expected_found_check(
            result,
            f"{name} payload rows",
            "TlsUserSpace OpenSSL payload rows with Complete and success",
            evidence_rows(
                payloads,
                [
                    (f"trace-{trace_id} OpenSSL TLS payload row", ("TlsUserSpace", "openssl", "Complete", "success")),
                ],
            ),
            "viewer payloads/events contain complete OpenSSL TLS rows and HTTP/2 Application rows",
        )
        add_expected_found_check(
            result,
            f"{name} application rows",
            "HTTP/2 Application frame or data rows",
            event_rows(
                events,
                [
                    (f"trace-{trace_id} HTTP/2 frame row", "Application", "frame", ()),
                    (f"trace-{trace_id} HTTP/2 data row", "Application", "data", ()),
                ],
            ),
            "local HTTP/2 docs path must derive Application rows from TLS plaintext",
        )
        return PASS
    except Exception as error:
        if isinstance(error, ViewEvidenceError):
            stop_process(daemon, workload)
            result.stdout_tail = env.output_tail(
                view_failure_transcript(error, process_artifact_transcript(env, daemon))
            )
            daemon = None
        return fail_step(env, result, name, error)
    finally:
        stop_process(server, workload)
        stop_process(daemon, workload)


def run_external_http1(env, result: CaseResult, workload: dict[str, str]) -> str:
    return run_external_llm_http(env, result, workload, "http1", "llm-http1", "http1.1")


def run_external_http2(env, result: CaseResult, workload: dict[str, str]) -> str:
    return run_external_llm_http(env, result, workload, "http2", "llm-http2", "http2")


def run_external_llm_http(
    env,
    result: CaseResult,
    workload: dict[str, str],
    suffix: str,
    clean_name: str,
    protocol: str,
) -> str:
    name = f"docs 02 external {protocol} payload"
    key_name = os.environ.get("ACTRAIL_LLM_API_KEY_ENV", "DEEPSEEK_API_KEY")
    if not os.environ.get(key_name):
        result.add_check(
            name,
            SKIP,
            evidence_detail(f"{key_name} present", f"{key_name} is not set"),
            "external OpenAI-compatible docs path requires provider credentials",
        )
        return SKIP
    config = None
    script = env.repo_root / f"docs/examples/02.llm-http-payload-capture/external-openai-compatible/{suffix}.sh"
    result.begin_check(name, "running provider curl workload")
    daemon = None
    launch_output = ""
    curl_tmpdir = None
    try:
        run_clean(env, clean_name, workload)
        clean_default_operator_state(env, workload)
        daemon = start_daemon(env, config, workload)
        record_process_artifacts(result, daemon)
        prepared = prepare_curl_files(env, script, workload)
        curl_tmpdir = prepared["ACTRAIL_CURL_TMPDIR"]
        launch = run_command(
            env,
            actrail_command(
                env,
                "actrailctl",
                config,
                "launch",
                "--name",
                f"docs-{clean_name}",
                "--",
                "curl",
                "--config",
                prepared["ACTRAIL_CURL_CONFIG"],
                "--data-binary",
                f"@{prepared['ACTRAIL_CURL_BODY']}",
            ),
            float(required(workload, "external_llm_launch_timeout_seconds")),
        )
        launch_output = launch.output
        trace_id = parse_trace_id(launch.output)
        curl_http_version = curl_http_version_line(launch_output)
        expected_curl_version = expected_curl_http_version(protocol)
        curl_version_status = curl_http_version_status(protocol, curl_http_version)
        add_expected_found_check(
            result,
            f"{name} curl negotiated HTTP version",
            expected_curl_http_version_detail(protocol, expected_curl_version),
            curl_http_version
            or "missing ACTRAIL_CURL_HTTP_VERSION marker in launch output",
            curl_http_version_evidence(protocol, curl_http_version),
            status=curl_version_status,
        )
        if curl_version_status == SKIP:
            result.stdout_tail = env.output_tail(launch_output)
            return SKIP
        if curl_version_status == FAIL:
            raise RuntimeError(
                f"{name} expected curl HTTP version {expected_curl_version}, got {curl_http_version}"
            )
        payloads, events, diagnostics = wait_payload_application_views(
            env,
            config,
            trace_id,
            workload,
            protocol,
            launch_output,
            require_wide_capture=True,
        )
        result.stdout_tail = env.output_tail("\n".join((payloads, events, diagnostics)))
        add_expected_found_check(
            result,
            f"{name} payload rows",
            "TlsUserSpace plaintext payload rows with Complete and success",
            evidence_rows(
                payloads,
                [
                    (f"trace-{trace_id} TLS plaintext payload row", ("TlsUserSpace", "Complete", "success")),
                ],
            ),
            "provider docs path must expose retained TLS plaintext payload rows",
        )
        if protocol == "http1.1":
            chat_path = os.environ.get("ACTRAIL_LLM_CHAT_PATH", "/chat/completions")
            expected = f"Application request POST {chat_path}"
            found = event_rows(
                events,
                [
                    (f"trace-{trace_id} HTTP/1.1 request row", "Application", "request", (f"POST {chat_path}",)),
                ],
            )
        else:
            expected = "HTTP/2 Application frame or data rows"
            found = event_rows(
                events,
                [
                    (f"trace-{trace_id} HTTP/2 frame row", "Application", "frame", ()),
                    (f"trace-{trace_id} HTTP/2 data row", "Application", "data", ()),
                ],
            )
        add_expected_found_check(
            result,
            f"{name} application rows",
            expected,
            found,
            "provider docs path must derive Application rows from captured plaintext",
        )
        add_expected_found_check(
            result,
            f"{name} wide capture rows",
            "Syscall socket payload rows and Stdio response rows are present as diagnostic side channels",
            evidence_rows(
                payloads,
                [
                    (f"trace-{trace_id} socket payload row", ("Syscall",)),
                    (f"trace-{trace_id} stdio payload row", ("Stdio",)),
                ],
            ),
            "provider docs config must collect enough side-channel evidence to debug TLS/Application misses",
        )
        return PASS
    except Exception as error:
        if isinstance(error, ViewEvidenceError):
            stop_process(daemon, workload)
            result.stdout_tail = env.output_tail(
                view_failure_transcript(error, process_artifact_transcript(env, daemon))
            )
            daemon = None
        return fail_step(env, result, name, error)
    finally:
        if curl_tmpdir:
            shutil.rmtree(curl_tmpdir, ignore_errors=True)
        stop_process(daemon, workload)


def parse_server_port(output: str) -> int:
    first_line = output.splitlines()[0] if output.splitlines() else ""
    try:
        return int(first_line)
    except ValueError as error:
        raise RuntimeError(f"local HTTP/2 server printed invalid port: {first_line!r}") from error


def wait_for_server_exit(server, workload: dict[str, str]) -> None:
    stdout, stderr = server.communicate(timeout=float(required(workload, "http2_local_launch_timeout_seconds")))
    if server.returncode != 0:
        raise RuntimeError(
            "local HTTP/2 server failed\n"
            f"stdout={stdout.decode(errors='replace')}\n"
            f"stderr={stderr.decode(errors='replace')}"
        )


def prepare_curl_files(env, script: Path, workload: dict[str, str]) -> dict[str, str]:
    prepared = run_command(
        env,
        ["bash", str(script), "prepare"],
        float(required(workload, "control_timeout_seconds")),
    )
    values: dict[str, str] = {}
    for line in prepared.stdout.splitlines():
        key, separator, raw_value = line.partition("=")
        if separator != "=":
            raise RuntimeError(f"invalid curl prepare output line: {line!r}")
        parts = shlex.split(raw_value)
        if len(parts) != 1:
            raise RuntimeError(f"invalid curl prepare value for {key}: {raw_value!r}")
        values[key] = parts[0]
    required_keys = {"ACTRAIL_CURL_CONFIG", "ACTRAIL_CURL_BODY", "ACTRAIL_CURL_TMPDIR"}
    missing = sorted(required_keys.difference(values))
    if missing:
        raise RuntimeError(f"curl prepare output missing keys: {', '.join(missing)}")
    return values


def wait_payload_application_views(
    env,
    config: Path | None,
    trace_id: int,
    workload: dict[str, str],
    protocol: str,
    launch_output: str,
    require_wide_capture: bool,
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
            socket_payload_line is not None and stdio_payload_line is not None
        )
        last_payload_ok = payload_ok
        last_event_ok = event_ok
        last_wide_capture_ok = wide_capture_ok
        last_payload_line = payload_line or "missing line containing TlsUserSpace, Complete, success"
        last_event_line = event_line or expected_event_missing_detail(protocol)
        last_socket_payload_line = socket_payload_line or "missing Syscall socket payload row"
        last_stdio_payload_line = stdio_payload_line or "missing Stdio payload row"
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
