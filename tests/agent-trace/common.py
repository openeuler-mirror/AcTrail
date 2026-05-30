#!/usr/bin/env python3
"""Shared helpers for real agent trace E2E cases."""

from __future__ import annotations

import json
import os
import re
import select
import signal
import subprocess
import sys
import time
from pathlib import Path


TRACE_RE = re.compile(r"trace trace-(\d+) entered Active")


def repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def read_config(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if not separator:
            raise RuntimeError(f"invalid config line in {path}: {raw}")
        values[key.strip()] = value.strip()
    return values


def required(values: dict[str, str], key: str) -> str:
    value = values.get(key)
    if not value:
        raise RuntimeError(f"missing config key {key}")
    return value


def require_root() -> None:
    if os.geteuid() != 0:
        raise RuntimeError("agent trace E2E requires root or equivalent eBPF/seccomp privileges")


def require_binary(bin_dir: Path, name: str) -> Path:
    path = bin_dir / name
    if not path.exists():
        raise RuntimeError(f"missing binary {path}; run cargo build --release")
    return path


def run_checked(
    command: list[str],
    *,
    echo: bool = True,
    timeout: float | None = None,
    cwd: Path | None = None,
) -> str:
    result = subprocess.run(
        command,
        text=True,
        capture_output=True,
        check=False,
        timeout=timeout,
        cwd=cwd,
    )
    if echo and result.stdout:
        print(result.stdout, end="", flush=True)
    if echo and result.stderr:
        print(result.stderr, end="", file=sys.stderr, flush=True)
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed: {' '.join(command)}\nstdout={result.stdout}\nstderr={result.stderr}"
        )
    return result.stdout


def render_config(template: Path, output: Path, replacements: dict[str, str]) -> None:
    raw = template.read_text(encoding="utf-8")
    for placeholder, value in replacements.items():
        if placeholder not in raw:
            raise RuntimeError(f"{template} does not contain {placeholder}")
        raw = raw.replace(placeholder, value)
    output.write_text(raw, encoding="utf-8")


def clean_configured_paths(actrailctl: Path, config: Path) -> None:
    run_checked([str(actrailctl), "--config", str(config), "clean"])


def start_daemon(actraild: Path, config: Path, timeout_sec: float) -> subprocess.Popen[str]:
    daemon = subprocess.Popen(
        [str(actraild), "--config", str(config), "run"],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    try:
        wait_for_daemon(daemon, timeout_sec)
    except Exception:
        stop_process(daemon, 5.0)
        raise
    return daemon


def wait_for_daemon(process: subprocess.Popen[str], timeout_sec: float) -> None:
    deadline = time.monotonic() + timeout_sec
    while time.monotonic() < deadline:
        line = read_line_until(process, process.stdout, deadline)
        if line:
            print(line, end="", flush=True)
            if "daemon listening" in line:
                return
        if process.poll() is not None:
            stderr = process.stderr.read() if process.stderr else ""
            raise RuntimeError(f"actraild exited early: {stderr}")
    raise RuntimeError("actraild did not report readiness")


def read_line_until(process: subprocess.Popen[str], stream, deadline: float) -> str:
    if stream is None:
        raise RuntimeError("process stream is not captured")
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        return ""
    readable, _, _ = select.select([stream], [], [], remaining)
    if readable:
        return stream.readline()
    if process.poll() is not None:
        return stream.readline()
    return ""


def launch_and_parse_trace(
    actrailctl: Path,
    config: Path,
    name: str,
    argv: list[str],
    timeout_sec: float,
) -> tuple[int, str]:
    command = [
        str(actrailctl),
        "--config",
        str(config),
        "launch",
        "--name",
        name,
        "--",
        *argv,
    ]
    output = run_checked(command, timeout=timeout_sec)
    match = TRACE_RE.search(output)
    if not match:
        raise RuntimeError(f"could not parse trace id from launch output: {output}")
    return int(match.group(1)), output


def launch_and_parse_trace_with_daemon(
    daemon: subprocess.Popen[str],
    actrailctl: Path,
    config: Path,
    name: str,
    argv: list[str],
    timeout_sec: float,
    poll_interval_sec: float,
    stop_timeout_sec: float,
) -> tuple[int, str]:
    require_positive_seconds(timeout_sec, "launch timeout")
    require_positive_seconds(poll_interval_sec, "launch poll interval")
    require_positive_seconds(stop_timeout_sec, "launch stop timeout")
    command = [
        str(actrailctl),
        "--config",
        str(config),
        "launch",
        "--name",
        name,
        "--",
        *argv,
    ]
    started = time.monotonic()
    process = subprocess.Popen(
        command,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    try:
        stdout, stderr, returncode = wait_for_launch(
            process,
            daemon,
            started,
            timeout_sec,
            poll_interval_sec,
            stop_timeout_sec,
        )
    except Exception:
        stop_process(process, stop_timeout_sec, process_group=True)
        raise
    if stdout:
        print(stdout, end="", flush=True)
    if stderr:
        print(stderr, end="", file=sys.stderr, flush=True)
    if returncode != 0:
        raise RuntimeError(
            f"command failed: {' '.join(command)}\nstdout={stdout}\nstderr={stderr}"
        )
    match = TRACE_RE.search(stdout)
    if not match:
        raise RuntimeError(f"could not parse trace id from launch output: {stdout}")
    return int(match.group(1)), f"{stdout}{stderr}"


def require_positive_seconds(value: float, name: str) -> None:
    if value <= 0.0:
        raise RuntimeError(f"{name} must be positive seconds")


def wait_for_launch(
    process: subprocess.Popen[str],
    daemon: subprocess.Popen[str],
    started: float,
    timeout_sec: float,
    poll_interval_sec: float,
    stop_timeout_sec: float,
) -> tuple[str, str, int]:
    deadline = started + timeout_sec
    while time.monotonic() < deadline:
        returncode = process.poll()
        if returncode is not None:
            stdout, stderr = process.communicate(timeout=stop_timeout_sec)
            return stdout, stderr, returncode
        if daemon.poll() is not None:
            stop_process(process, stop_timeout_sec, process_group=True)
            stdout, stderr = process.communicate(timeout=stop_timeout_sec)
            daemon_stdout = daemon.stdout.read() if daemon.stdout else ""
            daemon_stderr = daemon.stderr.read() if daemon.stderr else ""
            raise RuntimeError(
                "actraild exited while actrailctl launch was still running\n"
                f"elapsed_seconds={time.monotonic() - started:.1f}\n"
                f"daemon_status={daemon.returncode}\n"
                f"daemon_stdout={daemon_stdout}\n"
                f"daemon_stderr={daemon_stderr}\n"
                f"launch_stdout={stdout}\n"
                f"launch_stderr={stderr}"
            )
        time.sleep(poll_interval_sec)
    stop_process(process, stop_timeout_sec, process_group=True)
    stdout, stderr = process.communicate(timeout=stop_timeout_sec)
    raise RuntimeError(
        f"actrailctl launch timed out after {timeout_sec}s\nstdout={stdout}\nstderr={stderr}"
    )


def wait_for_payloads(
    actrailctl: Path,
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    attempts: int,
    sleep_sec: float,
    head: str,
    required_fragments: list[str],
) -> str:
    for _ in range(attempts):
        run_checked([str(actrailctl), "--config", str(config), "list-traces"], echo=False)
        output = run_checked(
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
        if all(fragment in output for fragment in required_fragments):
            print(output, end="", flush=True)
            return output
        time.sleep(sleep_sec)
    raise RuntimeError("viewer payload output missed required fragments")


def wait_for_payloads_any(
    actrailctl: Path,
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    attempts: int,
    sleep_sec: float,
    head: str,
    required_options: list[list[str]],
) -> str:
    for _ in range(attempts):
        run_checked([str(actrailctl), "--config", str(config), "list-traces"], echo=False)
        output = run_checked(
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
        if any(all(fragment in output for fragment in option) for option in required_options):
            print(output, end="", flush=True)
            return output
        time.sleep(sleep_sec)
    raise RuntimeError("viewer payload output missed every accepted fragment set")


def require_complete_payload_rows(payloads: str, source: str, library: str) -> int:
    count = 0
    for line in payloads.splitlines():
        if not re.match(r"^\s*payload-\d+\s+", line):
            continue
        if source not in line or library not in line:
            continue
        if "Truncated" in line or "success" not in line:
            raise RuntimeError(f"payload row is not complete/successful: {line}")
        count += 1
    if count == 0:
        raise RuntimeError(f"no {source} {library} payload rows found")
    return count


def require_complete_payload_rows_any(
    payloads: str,
    accepted: list[tuple[str, str]],
    direction: str | None = None,
) -> int:
    count = 0
    for line in payloads.splitlines():
        if not re.match(r"^\s*payload-\d+\s+", line):
            continue
        if direction is not None and direction not in line:
            continue
        if not any(source in line and library in line for source, library in accepted):
            continue
        if "Truncated" in line or "success" not in line:
            raise RuntimeError(f"payload row is not complete/successful: {line}")
        count += 1
    if count == 0:
        detail = ", ".join(f"{source} {library}" for source, library in accepted)
        direction_text = f" {direction}" if direction is not None else ""
        raise RuntimeError(f"no accepted{direction_text} payload rows found: {detail}")
    return count


def wait_for_actions(
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    attempts: int,
    sleep_sec: float,
) -> str:
    for _ in range(attempts):
        output = run_checked(
            [str(actrailviewer), "actions", "--config", str(config), "--trace-id", str(trace_id)],
            echo=False,
        )
        if "llm.request" in output:
            print(output, end="", flush=True)
            return output
        time.sleep(sleep_sec)
    raise RuntimeError("viewer actions did not show llm.request")


def require_complete_llm_action(actions: str) -> None:
    for line in actions.splitlines():
        if "llm.request" in line and "success" in line and "complete" in line:
            return
    raise RuntimeError("actions did not contain a complete successful llm.request")


def export_otel(actrailviewer: Path, config: Path, trace_id: int, output: Path) -> dict:
    if output.exists():
        output.unlink()
    run_checked(
        [
            str(actrailviewer),
            "export-otel",
            "--config",
            str(config),
            "--trace-id",
            str(trace_id),
            "--output",
            str(output),
        ],
        echo=False,
    )
    return json.loads(output.read_text(encoding="utf-8"))


def require_otel_span(document: dict, kind: str) -> int:
    count = 0
    for span in otel_spans(document):
        attrs = otel_attrs(span)
        if attrs.get("actrail.action.kind") == kind:
            count += 1
    if count == 0:
        raise RuntimeError(f"OTEL export did not contain {kind} span")
    return count


def emit_llm_otel_evidence(document: dict, max_text_chars: int) -> None:
    request = first_otel_action(document, "llm.request")
    if request is None:
        print("evidence.llm_request=not exported")
    else:
        attrs = otel_attrs(request)
        route = attrs.get("url.path", "")
        method = attrs.get("http.request.method", "")
        scheme = attrs.get("url.scheme", "")
        body = attrs.get("http.request.body_text") or attrs.get("llm.request.payload_text", "")
        print(f"evidence.llm_request.model={attrs.get('llm.request.model', '')}")
        print(f"evidence.llm_request.source={attrs.get('payload.source_boundary', '')}")
        print(f"evidence.llm_request.route={scheme} {method} {route}".rstrip())
        print(f"evidence.llm_request.payload_bytes={attrs.get('llm.request.payload_bytes', '')}")
        print(f"evidence.llm_request.body_text_json={json.dumps(clip_text(body, max_text_chars), ensure_ascii=False)}")

    response = first_otel_action(document, "llm.response")
    if response is None:
        print("evidence.llm_response=not exported")
    else:
        attrs = otel_attrs(response)
        body = attrs.get("http.response.body_text") or attrs.get("llm.response.payload_text", "")
        print(f"evidence.llm_response.model={attrs.get('llm.response.model', '')}")
        print(f"evidence.llm_response.source={attrs.get('payload.source_boundary', '')}")
        print(f"evidence.llm_response.payload_bytes={attrs.get('llm.response.payload_bytes', '')}")
        print(f"evidence.llm_response.body_text_json={json.dumps(clip_text(body, max_text_chars), ensure_ascii=False)}")


def first_otel_action(document: dict, kind: str) -> dict | None:
    for span in otel_spans(document):
        attrs = otel_attrs(span)
        if attrs.get("actrail.action.kind") == kind:
            return span
    return None


def clip_text(text: str, max_chars: int) -> str:
    if max_chars < 0:
        raise RuntimeError("evidence text max chars must be non-negative")
    return text if len(text) <= max_chars else text[:max_chars] + "...[truncated]"


def otel_spans(document: dict) -> list[dict]:
    spans: list[dict] = []
    for resource in document.get("resourceSpans", []):
        for scope in resource.get("scopeSpans", []):
            spans.extend(scope.get("spans", []))
    return spans


def otel_attrs(span: dict) -> dict[str, str]:
    attrs: dict[str, str] = {}
    for attr in span.get("attributes", []):
        value = attr.get("value", {})
        if "stringValue" in value:
            attrs[attr.get("key", "")] = value["stringValue"]
        elif "intValue" in value:
            attrs[attr.get("key", "")] = str(value["intValue"])
    return attrs


def stop_process(
    process: subprocess.Popen[str],
    timeout_sec: float,
    process_group: bool = False,
) -> None:
    if process.poll() is not None:
        return
    signal_process(process, process_group, signal.SIGTERM)
    try:
        process.wait(timeout=timeout_sec)
    except subprocess.TimeoutExpired:
        signal_process(process, process_group, signal.SIGKILL)
        process.wait(timeout=timeout_sec)


def signal_process(
    process: subprocess.Popen[str],
    process_group: bool,
    sig: signal.Signals,
) -> None:
    try:
        if process_group:
            os.killpg(process.pid, sig)
        else:
            process.send_signal(sig)
    except ProcessLookupError:
        return
