"""Docs example 02 HTTP payload regression steps."""

from __future__ import annotations

import os
import shlex
import shutil
from pathlib import Path

from model import FAIL, PASS, SKIP, CaseResult
from workload_config import required

from helpers import (
    add_expected_found_check,
    actrail_command,
    clean_default_operator_state,
    evidence_detail,
    evidence_rows,
    event_rows,
    fail_step,
    operator_config_path,
    parse_trace_id,
    process_artifact_transcript,
    record_process_artifacts,
    run_clean,
    run_command,
    start_daemon,
    start_process,
    stop_process,
    wait_for_output,
)
from case_02_llm_http_payload_capture.stdio_policy import curl_stdout_payload_is_stored_for_operator_config
from case_02_llm_http_payload_capture.view_checks import (
    ViewEvidenceError,
    curl_http_version_evidence,
    curl_http_version_line,
    curl_http_version_status,
    expected_curl_http_version,
    expected_curl_http_version_detail,
    view_failure_transcript,
    wait_payload_application_views,
    wide_capture_expected,
    wide_capture_reason,
)


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
            require_stdio_payload=False,
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
        require_stdio_payload = curl_stdout_payload_is_stored_for_operator_config(operator_config_path(env, config))
        payloads, events, diagnostics = wait_payload_application_views(
            env,
            config,
            trace_id,
            workload,
            protocol,
            launch_output,
            require_wide_capture=True,
            require_stdio_payload=require_stdio_payload,
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
        wide_capture_rows = [(f"trace-{trace_id} socket payload row", ("Syscall",))]
        if require_stdio_payload:
            wide_capture_rows.append((f"trace-{trace_id} stdio payload row", ("Stdio",)))
        add_expected_found_check(
            result,
            f"{name} wide capture rows",
            wide_capture_expected(require_stdio_payload),
            evidence_rows(payloads, wide_capture_rows),
            wide_capture_reason(require_stdio_payload),
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
