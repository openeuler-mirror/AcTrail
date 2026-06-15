#!/usr/bin/env python3
"""Run hidden agent invocation E2E with real LLM calls."""

from __future__ import annotations

import argparse
import json
import os
import re
import select
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

from runner.otel import (
    describe_hidden_agent_evidence,
    hidden_agent_evidence_is_complete,
    validate_hidden_agent_actions,
)


TRACE_RE = re.compile(r"trace trace-(\d+) entered Active")


def main() -> int:
    args = parse_args()
    success = False
    require_root()
    repo = Path.cwd()
    bin_dir = repo / args.bin_dir
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
    config = Path(args.config)
    workload_path = Path(args.workload_config)
    workload = read_config(workload_path)
    require_command("xiaoo")
    require_command("gcc")
    require_env(required(workload, "api_key_env"))
    prepare_configured_directories(config)
    clean_configured_paths(actrailctl, config)
    prepare_configured_directories(config)
    agent_a_binary = compile_agent_a(workload, workload_path)
    daemon = subprocess.Popen(
        [str(actraild), "--config", str(config), "run"],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    try:
        wait_for_daemon(daemon, float(required(workload, "daemon_ready_timeout_seconds")))
        launch = run_hidden_workload(
            daemon,
            actrailctl,
            config,
            workload_path,
            workload,
            agent_a_binary,
            float(required(workload, "launch_timeout_seconds")),
            float(required(workload, "launch_poll_interval_seconds")),
            float(required(workload, "process_stop_timeout_seconds")),
        )
        require_output(launch.output, required(workload, "expected_a_output"))
        require_output(launch.output, required(workload, "expected_xiaoo_output"))
        output_path = Path(required(workload, "otel_output_path"))
        otel = wait_for_otel(
            actrailctl,
            actrailviewer,
            config,
            launch.trace_id,
            output_path,
            int(required(workload, "drain_attempts")),
            float(required(workload, "drain_sleep_seconds")),
            required(workload, "script_b_path"),
            str(agent_a_binary),
        )
        proof = validate_hidden_agent_actions(
            otel,
            required(workload, "script_b_path"),
            str(agent_a_binary),
        )
        print(f"hidden_agent_trace_id={launch.trace_id}")
        print(f"agent_a_pid={proof.agent_a_pid}")
        print(f"xiaoo_pid={proof.xiaoo_pid}")
        print(f"script_b_parent_pid={proof.script_b_parent_pid}")
        print("hidden agent invocation e2e complete")
        output_path.unlink(missing_ok=True)
        success = True
    finally:
        stop_process(daemon, float(required(workload, "daemon_stop_timeout_seconds")))
        if success or not args.keep_artifacts_on_failure:
            clean_configured_paths(actrailctl, config)
            cleanup_helper_binary(workload)
    return 0


def parse_args() -> argparse.Namespace:
    test_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--config", default=str(test_dir / "operator.conf"))
    parser.add_argument("--workload-config", default=str(test_dir / "workload.conf"))
    parser.add_argument("--keep-artifacts-on-failure", action="store_true")
    return parser.parse_args()


def run_hidden_workload(
    daemon: subprocess.Popen[str],
    actrailctl: Path,
    config: Path,
    workload_path: Path,
    workload: dict[str, str],
    agent_a: Path,
    timeout_sec: float,
    poll_interval_sec: float,
    stop_timeout_sec: float,
) -> "LaunchResult":
    script_b = resolve_config_path(required(workload, "script_b_path"), workload_path)
    command = [
        str(actrailctl),
        "--config",
        str(config),
        "launch",
        "--name",
        "hidden-agent-invocation",
        "--",
        str(agent_a),
        "--api-key-env",
        required(workload, "api_key_env"),
        "--api-host",
        required(workload, "api_host"),
        "--api-port",
        required(workload, "api_port"),
        "--api-path",
        required(workload, "api_path"),
        "--model",
        required(workload, "model"),
        "--agent-a-prompt",
        required(workload, "agent_a_prompt"),
        "--script-b",
        str(script_b),
        "--xiaoo-prompt",
        required(workload, "xiaoo_prompt"),
        "--xiaoo-provider",
        required(workload, "xiaoo_provider"),
        "--xiaoo-model",
        required(workload, "xiaoo_model"),
        "--xiaoo-max-turns",
        required(workload, "xiaoo_max_turns"),
        "--xiaoo-no-tools",
        required(workload, "xiaoo_no_tools"),
        "--child-timeout-seconds",
        required(workload, "child_timeout_seconds"),
        "--child-poll-interval-millis",
        required(workload, "child_poll_interval_millis"),
        "--io-chunk-bytes",
        required(workload, "agent_a_io_chunk_bytes"),
        "--expected-a-output",
        required(workload, "expected_a_output"),
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
    match = TRACE_RE.search(stdout)
    if not match:
        raise RuntimeError(f"could not parse trace id from launch output: {stdout}")
    print(f"launch_seconds={time.monotonic() - started:.1f}", flush=True)
    return LaunchResult(trace_id=int(match.group(1)), output=f"{stdout}{stderr}")


def wait_for_otel(
    actrailctl: Path,
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    output_path: Path,
    attempts: int,
    sleep_sec: float,
    script_b_path: str,
    agent_a_path: str,
) -> dict:
    last_document = None
    for _ in range(attempts):
        run_checked([str(actrailctl), "--config", str(config), "list-traces"], echo=False)
        output_path.unlink(missing_ok=True)
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
            last_document = document
            if hidden_agent_evidence_is_complete(document, script_b_path, agent_a_path):
                return document
        time.sleep(sleep_sec)
    summary = "no successful OTEL export"
    if last_document is not None:
        summary = describe_hidden_agent_evidence(last_document, script_b_path, agent_a_path)
    raise RuntimeError(
        "OTEL export did not contain complete hidden agent evidence\n" + summary
    )


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


def wait_for_daemon(process: subprocess.Popen[str], timeout_sec: float) -> None:
    started = time.monotonic()
    deadline = time.monotonic() + timeout_sec
    while time.monotonic() < deadline:
        line = read_line_until(process, process.stdout, deadline)
        if line:
            print(line, end="", flush=True)
            if "daemon listening" in line:
                print(f"daemon_ready_seconds={time.monotonic() - started:.1f}", flush=True)
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


def clean_configured_paths(actrailctl: Path, config: Path) -> None:
    run_checked([str(actrailctl), "--config", str(config), "clean"])


def compile_agent_a(workload: dict[str, str], workload_path: Path) -> Path:
    source = resolve_config_path(required(workload, "agent_a_source_path"), workload_path)
    binary = Path(required(workload, "agent_a_binary_path"))
    if not source.exists():
        raise RuntimeError(f"missing agent A source: {source}")
    ensure_parent_directory(binary)
    result = subprocess.run(
        [
            "gcc",
            str(source),
            "-o",
            str(binary),
            "-lssl",
            "-lcrypto",
        ],
        text=True,
        capture_output=True,
        timeout=float(required(workload, "agent_a_compile_timeout_seconds")),
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"compile agent A failed with status {result.returncode}\n"
            f"stdout={result.stdout}\nstderr={result.stderr}"
        )
    return binary


def cleanup_helper_binary(workload: dict[str, str]) -> None:
    binary = Path(required(workload, "agent_a_binary_path"))
    binary.unlink(missing_ok=True)
    runtime_dir = binary.parent
    output_path = Path(required(workload, "otel_output_path"))
    output_path.unlink(missing_ok=True)
    try:
        runtime_dir.rmdir()
    except OSError:
        return


def prepare_configured_directories(config: Path) -> None:
    values = read_config(config)
    for key in (
        "socket_path",
        "pid_file",
        "storage_sqlite_path",
        "log_path",
        "export_otel_jsonl_path",
        "payload_tls_sync_event_socket_path",
        "enforcement_rules_path",
    ):
        ensure_parent_directory(Path(required(values, key)))
    Path(required(values, "export_directory")).mkdir(parents=True, exist_ok=True)


def ensure_parent_directory(path: Path) -> None:
    parent = path.parent
    if parent != Path("."):
        parent.mkdir(parents=True, exist_ok=True)


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


def read_config(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    section = ""
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            section = line.strip("[]")
            continue
        key, separator, value = line.partition("=")
        if not separator:
            raise RuntimeError(f"invalid config line: {raw}")
        key = key.strip()
        value = unquote(value.strip())
        if section == "export" and key == "enabled":
            values["export_enabled"] = value
            continue
        if section.startswith("export.routes.otel-jsonl.") and key == "path":
            values["export_otel_jsonl_path"] = value
            continue
        if section.startswith("export."):
            continue
        values[key] = value
    return values


def unquote(value: str) -> str:
    if len(value) >= 2 and value.startswith('"') and value.endswith('"'):
        return value[1:-1]
    return value


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


def require_root() -> None:
    if os.geteuid() != 0:
        raise RuntimeError("hidden agent E2E requires root for eBPF/seccomp setup")


def require_binary(bin_dir: Path, name: str) -> Path:
    path = bin_dir / name
    if not path.exists():
        raise RuntimeError(f"missing binary {path}; build with cargo build --release")
    return path


def require_command(name: str) -> None:
    if shutil.which(name) is None:
        raise RuntimeError(f"{name} is not on PATH")


def require_env(name: str) -> None:
    if not os.environ.get(name):
        raise RuntimeError(f"missing environment variable {name}")


def require_output(output: str, expected: str) -> None:
    if expected not in output:
        raise RuntimeError(f"launch output did not contain {expected}")


class LaunchResult:
    def __init__(self, trace_id: int, output: str) -> None:
        self.trace_id = trace_id
        self.output = output


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"hidden agent invocation e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
