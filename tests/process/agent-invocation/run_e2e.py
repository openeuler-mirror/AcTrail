#!/usr/bin/env python3
"""Run a real xiaoO -> claude invocation E2E and verify pretty OTEL output."""

from __future__ import annotations

import argparse
import json
import os
import re
import select
import signal
import shutil
import subprocess
import sys
import time
from pathlib import Path


TRACE_RE = re.compile(r"trace trace-(\d+) entered Active")


def main() -> int:
    args = parse_args()
    require_root()
    bin_dir = Path.cwd() / args.bin_dir
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
    config = Path(args.config)
    workload_path = Path(args.workload_config)
    workload = read_config(workload_path)
    agent_command = resolve_agent_command(required(workload, "agent_command"))
    require_command("claude")
    prompt = render_workload_prompt(workload, workload_path)
    clean_configured_paths(actrailctl, config)
    daemon = subprocess.Popen(
        [str(actraild), "--config", str(config), "run"],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    try:
        wait_for_daemon(daemon, float(required(workload, "daemon_ready_timeout_seconds")))
        launch = run_agent_invocation(
            daemon,
            actrailctl,
            config,
            agent_command,
            prompt,
            float(required(workload, "launch_timeout_seconds")),
            float(required(workload, "launch_poll_interval_seconds")),
            float(required(workload, "process_stop_timeout_seconds")),
        )
        expected_output = required(workload, "expected_output_fragment")
        if expected_output not in launch.output:
            raise RuntimeError(f"xiaoO/claude output did not contain {expected_output}")
        otel = wait_for_otel(
            actrailctl,
            actrailviewer,
            config,
            launch.trace_id,
            Path(required(workload, "otel_output_path")),
            int(required(workload, "drain_attempts")),
            float(required(workload, "drain_sleep_seconds")),
        )
        require_claude_exec_span(otel)
        require_agent_invocation_span(otel)
        print(f"agent_invocation_trace_id={launch.trace_id}")
        print(f"otel_output={required(workload, 'otel_output_path')}")
        print("agent invocation e2e complete")
    finally:
        stop_process(daemon, float(required(workload, "daemon_stop_timeout_seconds")))
    return 0


def parse_args() -> argparse.Namespace:
    test_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--config", default=str(test_dir / "operator.conf"))
    parser.add_argument("--workload-config", default=str(test_dir / "workload.conf"))
    return parser.parse_args()


def require_root() -> None:
    if os.geteuid() != 0:
        raise RuntimeError("process seccomp E2E requires root for eBPF/seccomp setup")


def require_binary(bin_dir: Path, name: str) -> Path:
    path = bin_dir / name
    if not path.exists():
        raise RuntimeError(f"missing binary {path}; build with cargo build --release")
    return path


def require_command(name: str) -> None:
    if shutil.which(name) is None:
        raise RuntimeError(f"{name} is not on PATH")


def resolve_agent_command(raw: str) -> str:
    path = Path(raw)
    if path.parent != Path("."):
        if not path.exists():
            raise RuntimeError(f"configured agent_command does not exist: {raw}")
        if not os.access(path, os.X_OK):
            raise RuntimeError(f"configured agent_command is not executable: {raw}")
        return str(path)
    resolved = shutil.which(raw)
    if resolved is None:
        raise RuntimeError(f"configured agent_command is not on PATH: {raw}")
    return resolved


def read_config(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if not separator:
            raise RuntimeError(f"invalid config line: {raw}")
        values[key.strip()] = value.strip()
    return values


def render_workload_prompt(values: dict[str, str], workload_path: Path) -> str:
    template_path = resolve_config_path(
        required(values, "agent_prompt_template"),
        workload_path,
    )
    template = template_path.read_text(encoding="utf-8").strip()
    try:
        return template.format_map(values)
    except KeyError as error:
        raise RuntimeError(f"missing prompt template value: {error}") from error


def resolve_config_path(raw: str, workload_path: Path) -> Path:
    path = Path(raw)
    if path.is_absolute():
        return path
    cwd_path = Path.cwd() / path
    if cwd_path.exists():
        return cwd_path
    return workload_path.resolve().parent / path


def required(values: dict[str, str], key: str) -> str:
    value = values.get(key)
    if not value:
        raise RuntimeError(f"missing config key {key}")
    return value


def clean_configured_paths(actrailctl: Path, config: Path) -> None:
    run_checked([str(actrailctl), "--config", str(config), "clean"])


def wait_for_daemon(process: subprocess.Popen[str], timeout_sec: float) -> None:
    started = time.monotonic()
    deadline = time.monotonic() + timeout_sec
    while time.monotonic() < deadline:
        line = read_line_until(process, process.stdout, deadline)
        if line:
            print(line, end="", flush=True)
            if "daemon listening" in line:
                print(
                    f"daemon_ready_seconds={time.monotonic() - started:.1f}",
                    flush=True,
                )
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


def run_agent_invocation(
    daemon: subprocess.Popen[str],
    actrailctl: Path,
    config: Path,
    agent_command: str,
    prompt: str,
    timeout_sec: float,
    poll_interval_sec: float,
    stop_timeout_sec: float,
) -> LaunchResult:
    command = [
        str(actrailctl),
        "--config",
        str(config),
        "launch",
        "--name",
        "xiaoo-calls-claude",
        "--",
        agent_command,
        "run",
        "-p",
        prompt,
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
    print(stdout, end="", flush=True)
    if stderr:
        print(stderr, end="", file=sys.stderr, flush=True)
    if returncode != 0:
        raise RuntimeError(f"actrailctl launch exited with status {returncode}")
    print(f"launch_seconds={time.monotonic() - started:.1f}", flush=True)
    match = TRACE_RE.search(stdout)
    if not match:
        raise RuntimeError(f"could not parse trace id from launch output: {stdout}")
    return LaunchResult(trace_id=int(match.group(1)), output=f"{stdout}{stderr}")


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
            daemon_stdout = daemon.stdout.read() if daemon.stdout else ""
            daemon_stderr = daemon.stderr.read() if daemon.stderr else ""
            raise RuntimeError(
                "actraild exited while actrailctl launch was still running\n"
                f"elapsed_seconds={time.monotonic() - started:.1f}\n"
                f"daemon_status={daemon.returncode}\n"
                f"daemon_stdout={daemon_stdout}\n"
                f"daemon_stderr={daemon_stderr}"
            )
        time.sleep(poll_interval_sec)
    stop_process(process, stop_timeout_sec, process_group=True)
    stdout, stderr = process.communicate(timeout=stop_timeout_sec)
    raise RuntimeError(
        f"actrailctl launch timed out after {timeout_sec}s\nstdout={stdout}\nstderr={stderr}"
    )


def wait_for_otel(
    actrailctl: Path,
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    output_path: Path,
    attempts: int,
    sleep_sec: float,
) -> dict:
    for _ in range(attempts):
        run_checked([str(actrailctl), "--config", str(config), "list-traces"], echo=False)
        if output_path.exists():
            output_path.unlink()
        result = subprocess.run(
            [
                str(actrailviewer),
                "--config",
                str(config),
                "export-otel",
                "--trace-id",
                str(trace_id),
                "--output",
                str(output_path),
            ],
            text=True,
            capture_output=True,
            check=False,
        )
        if result.returncode == 0 and output_path.exists():
            document = json.loads(output_path.read_text(encoding="utf-8"))
            if find_claude_exec_span(document) is not None:
                return document
        time.sleep(sleep_sec)
    raise RuntimeError("OTEL export did not contain a seccomp-observed claude process.exec span")


def require_claude_exec_span(document: dict) -> None:
    span = find_claude_exec_span(document)
    if span is None:
        raise RuntimeError("missing claude process.exec span")
    attrs = span_attrs(span)
    if attrs.get("seccomp_observed") != "true":
        raise RuntimeError("claude process.exec span was not marked seccomp_observed=true")
    command_line = attrs.get("command_line", "")
    if "claude" not in command_line or "-p" not in command_line:
        raise RuntimeError(f"claude command_line missing expected argv: {command_line}")


def require_agent_invocation_span(document: dict) -> None:
    span = find_claude_agent_invocation_span(document)
    if span is None:
        raise RuntimeError("missing xiaoO -> claude agent.invocation span")
    attrs = span_attrs(span)
    parent = attrs.get("agent.parent.executable", "")
    child = attrs.get("agent.child.command_line", "")
    if executable_basename(parent) != "xiaoo":
        raise RuntimeError(f"agent invocation parent is not xiaoO: {parent}")
    if "claude" not in child:
        raise RuntimeError(f"agent invocation child is not claude: {child}")


def find_claude_exec_span(document: dict) -> dict | None:
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "process.exec":
            continue
        if attrs.get("seccomp_observed") != "true":
            continue
        if not is_claude_prompt_exec(span, attrs):
            continue
        return span
    return None


def is_claude_prompt_exec(span: dict, attrs: dict[str, str]) -> bool:
    command_line = attrs.get("command_line", "")
    argv = attrs.get("argv", "")
    if "-p" not in command_line and "\n-p\n" not in argv:
        return False
    executable_candidates = [
        span.get("name", ""),
        attrs.get("process.executable", ""),
        attrs.get("executable", ""),
        attrs.get("exec.path", ""),
    ]
    if any(executable_basename(value) == "claude" for value in executable_candidates):
        return True
    return "\nclaude\n-p" in argv or "/claude\n-p" in argv


def find_claude_agent_invocation_span(document: dict) -> dict | None:
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "agent.invocation":
            continue
        child_executable = attrs.get("agent.child.executable", "")
        child_command = attrs.get("agent.child.command_line", "")
        if executable_basename(child_executable) != "claude":
            continue
        if "-p" not in child_command:
            continue
        return span
    return None


def executable_basename(value: str) -> str:
    return Path(value).name if value else ""


def spans(document: dict) -> list[dict]:
    result: list[dict] = []
    for resource in document.get("resourceSpans", []):
        for scope in resource.get("scopeSpans", []):
            result.extend(scope.get("spans", []))
    return result


def span_attrs(span: dict) -> dict[str, str]:
    attrs: dict[str, str] = {}
    for attr in span.get("attributes", []):
        value = attr.get("value", {})
        if "stringValue" in value:
            attrs[attr.get("key", "")] = value["stringValue"]
        elif "intValue" in value:
            attrs[attr.get("key", "")] = str(value["intValue"])
    return attrs


def run_checked(command: list[str], echo: bool = True) -> str:
    result = subprocess.run(command, text=True, capture_output=True, check=False)
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed: {' '.join(command)}\nstdout={result.stdout}\nstderr={result.stderr}"
        )
    if echo and result.stdout:
        print(result.stdout, end="", flush=True)
    if echo and result.stderr:
        print(result.stderr, end="", file=sys.stderr, flush=True)
    return result.stdout


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


class LaunchResult:
    def __init__(self, trace_id: int, output: str) -> None:
        self.trace_id = trace_id
        self.output = output


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"agent invocation e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
