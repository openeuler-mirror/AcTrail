#!/usr/bin/env python3
"""Run the AcTrail HTTP socket payload E2E."""

from __future__ import annotations

import argparse
import os
import re
import select
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path


TRACE_RE = re.compile(r"trace trace-(\d+) entered Active")
DEFAULT_OPERATOR_CONFIG_PATH = Path("/etc/actrail/actraild.conf")
SINGLE_VALUE_CONFIG_KEYS = {
    "socket_path",
    "pid_file",
    "storage_path",
    "log_path",
    "export_directory",
    "otel_live_export_path",
    "payload_tls_sync_event_socket_path",
}


def parse_args() -> argparse.Namespace:
    example_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--config", default=str(example_dir / "operator.conf"))
    parser.add_argument("--default-config", action="store_true", help="use the CLI default operator config")
    parser.add_argument("--workload", default=str(example_dir / "workload.py"))
    parser.add_argument("--daemon-ready-timeout-sec", type=float, default=10.0)
    parser.add_argument("--workload-timeout-sec", type=float, default=10.0)
    parser.add_argument("--drain-attempts", type=int, default=25)
    parser.add_argument("--drain-sleep-sec", type=float, default=0.2)
    parser.add_argument("--events-head", type=int, default=800)
    parser.add_argument("--payload-head", type=int, default=80)
    return parser.parse_args()


def operator_config_path(config: Path | None) -> Path:
    return config if config is not None else DEFAULT_OPERATOR_CONFIG_PATH


def actrail_command(binary: Path, config: Path | None, *args: str) -> list[str]:
    command = [str(binary)]
    if config is not None:
        command.extend(["--config", str(config)])
    command.extend(args)
    return command


def require_root() -> None:
    if os.geteuid() != 0:
        raise RuntimeError("HTTP socket payload eBPF capture requires root/CAP_BPF")


def require_binary(bin_dir: Path, name: str) -> Path:
    path = bin_dir / name
    if not path.exists():
        raise RuntimeError(f"missing binary {path}; build with cargo build --release")
    return path


def read_operator_config(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if not separator:
            continue
        key = key.strip()
        if key in values and key in SINGLE_VALUE_CONFIG_KEYS:
            raise RuntimeError(f"duplicate config key {key} in {path}")
        values.setdefault(key, value.strip())
    return values


def clean_configured_paths(values: dict[str, str]) -> None:
    for key in [
        "socket_path",
        "pid_file",
        "storage_path",
        "log_path",
        "payload_tls_sync_event_socket_path",
    ]:
        path = Path(required_value(values, key))
        if path.exists():
            path.unlink()
    export_dir = Path(required_value(values, "export_directory"))
    if export_dir.exists():
        shutil.rmtree(export_dir)
    if values.get("otel_live_export_enabled") == "true":
        live_otel_path = Path(required_value(values, "otel_live_export_path"))
        if live_otel_path.exists():
            live_otel_path.unlink()


def required_value(values: dict[str, str], key: str) -> str:
    value = values.get(key)
    if not value:
        raise RuntimeError(f"missing required config key {key}")
    return value


def wait_for_daemon(process: subprocess.Popen[str], timeout_sec: float) -> None:
    deadline = time.monotonic() + timeout_sec
    while time.monotonic() < deadline:
        line = read_line_until(process, process.stdout, deadline)
        if line:
            print(line, end="")
            if "daemon listening" in line:
                return
        if process.poll() is not None:
            stderr = process.stderr.read() if process.stderr else ""
            raise RuntimeError(f"actraild exited early: {stderr}")
    raise RuntimeError("actraild did not report readiness")


def read_agent_pid(process: subprocess.Popen[str], timeout_sec: float) -> int:
    deadline = time.monotonic() + timeout_sec
    while time.monotonic() < deadline:
        line = read_line_until(process, process.stdout, deadline)
        if line:
            print(line, end="")
            if line.startswith("agent_pid="):
                return int(line.split("=", 1)[1])
        if process.poll() is not None:
            raise RuntimeError("workload exited before reporting agent_pid")
    raise RuntimeError("workload did not report agent_pid")


def read_launch_ready(process: subprocess.Popen[str], timeout_sec: float) -> tuple[int, str]:
    if process.stdout is None:
        raise RuntimeError("launch stdout is not captured")
    stdout_fd = process.stdout.fileno()
    os.set_blocking(stdout_fd, False)
    deadline = time.monotonic() + timeout_sec
    trace_id = None
    output = bytearray()
    try:
        while time.monotonic() < deadline:
            remaining = deadline - time.monotonic()
            readable, _, _ = select.select([stdout_fd], [], [], max(0.0, remaining))
            if readable:
                chunk = os.read(stdout_fd, 4096)
                if chunk:
                    output.extend(chunk)
                    text = output.decode("utf-8", errors="replace")
                    print(chunk.decode("utf-8", errors="replace"), end="")
                    match = TRACE_RE.search(text)
                    if match:
                        trace_id = int(match.group(1))
                    if "waiting_for=go" in text:
                        if trace_id is None:
                            raise RuntimeError("launch workload was ready before trace id was reported")
                        return trace_id, text
            if process.poll() is not None:
                raise RuntimeError("launch exited before workload was ready")
        raise RuntimeError("launch workload did not report readiness")
    finally:
        os.set_blocking(stdout_fd, True)



def read_line_until(
    process: subprocess.Popen[str],
    stream,
    deadline: float,
) -> str:
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


def run_checked(command: list[str], echo: bool = True) -> str:
    result = subprocess.run(command, text=True, capture_output=True, check=False)
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed: {' '.join(command)}\nstdout={result.stdout}\nstderr={result.stderr}"
        )
    if echo and result.stdout:
        print(result.stdout, end="")
    if echo and result.stderr:
        print(result.stderr, end="", file=sys.stderr)
    return result.stdout


def attach_trace(actrailctl: Path, config: Path | None, pid: int) -> int:
    output = run_checked(
        actrail_command(
            actrailctl,
            config,
            "track-add",
            "--pid",
            str(pid),
            "--name",
            "http-payload-unified-e2e",
        )
    )
    match = TRACE_RE.search(output)
    if not match:
        raise RuntimeError(f"could not parse trace id from actrailctl output: {output}")
    return int(match.group(1))


def release_workload(process: subprocess.Popen[str], timeout_sec: float) -> str:
    if process.stdin is None:
        raise RuntimeError("workload stdin is not captured")
    process.stdin.write("go\n")
    process.stdin.flush()
    try:
        stdout, stderr = process.communicate(timeout=timeout_sec)
    except subprocess.TimeoutExpired as error:
        process.kill()
        stdout, stderr = process.communicate()
        raise RuntimeError(f"workload timed out\nstdout={stdout}\nstderr={stderr}") from error
    print(stdout, end="")
    if stderr:
        print(stderr, end="", file=sys.stderr)
    if process.returncode != 0:
        raise RuntimeError(f"workload failed with exit={process.returncode}")
    if "workload complete" not in stdout:
        raise RuntimeError("workload did not report completion")
    return stdout


def release_launched_workload(
    process: subprocess.Popen[str],
    prefix_stdout: str,
    timeout_sec: float,
) -> str:
    if process.stdin is None:
        raise RuntimeError("launch stdin is not captured")
    process.stdin.write("go\n")
    process.stdin.flush()
    try:
        stdout, stderr = process.communicate(timeout=timeout_sec)
    except subprocess.TimeoutExpired as error:
        process.kill()
        stdout, stderr = process.communicate()
        raise RuntimeError(f"launch workload timed out\nstdout={prefix_stdout}{stdout}\nstderr={stderr}") from error
    if stdout:
        print(stdout, end="")
    if stderr:
        print(stderr, end="", file=sys.stderr)
    combined_stdout = prefix_stdout + stdout
    if process.returncode != 0:
        raise RuntimeError(f"launch workload failed with exit={process.returncode}")
    if "workload complete" not in combined_stdout:
        raise RuntimeError("launch workload did not report completion")
    return combined_stdout


def wait_for_payload_and_events(
    actrailctl: Path,
    actrailviewer: Path,
    config: Path | None,
    trace_id: int,
    attempts: int,
    sleep_sec: float,
    events_head: int,
    payload_head: int,
) -> tuple[str, str]:
    for _ in range(attempts):
        run_checked(actrail_command(actrailctl, config, "list-traces"), echo=False)
        events = run_checked(
            actrail_command(
                actrailviewer,
                config,
                "events",
                "--trace-id",
                str(trace_id),
                "--head",
                str(events_head),
            ),
            echo=False,
        )
        payloads = run_checked(
            actrail_command(
                actrailviewer,
                config,
                "payloads",
                "--trace-id",
                str(trace_id),
                "--head",
                str(payload_head),
            ),
            echo=False,
        )
        if all(value in events for value in ["Application", "POST /plain-http", "200 OK"]):
            if "Syscall" in payloads and ("sendto" in payloads or "recvfrom" in payloads):
                print(events, end="")
                print(payloads, end="")
                return events, payloads
        time.sleep(sleep_sec)
    raise RuntimeError("viewer did not show expected HTTP socket payload/Application rows")


def payload_texts(actrailviewer: Path, config: Path | None, trace_id: int, payloads: str) -> str:
    segment_ids = parse_segment_ids(payloads)
    if not segment_ids:
        raise RuntimeError("payloads output did not contain any segment ids")
    texts: list[str] = []
    for segment_id in segment_ids:
        texts.append(
            run_checked(
                actrail_command(
                    actrailviewer,
                    config,
                    "payload",
                    "--trace-id",
                    str(trace_id),
                    "--segment-id",
                    segment_id,
                    "--format",
                    "text",
                )
            )
        )
    return "\n".join(texts)


def parse_segment_ids(payloads: str) -> list[str]:
    ids: list[str] = []
    for line in payloads.splitlines():
        match = re.match(r"^\s*(payload-\d+)\s+", line)
        if match:
            ids.append(match.group(1))
    return ids


def stop_process(process: subprocess.Popen[str]) -> None:
    if process.poll() is not None:
        return
    process.send_signal(signal.SIGTERM)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def main() -> int:
    args = parse_args()
    require_root()
    repo = Path.cwd()
    bin_dir = repo / args.bin_dir
    config = None if args.default_config else Path(args.config)
    workload = Path(args.workload)
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
    if not workload.exists():
        raise RuntimeError(f"missing workload script {workload}")

    values = read_operator_config(operator_config_path(config))
    clean_configured_paths(values)

    daemon = subprocess.Popen(
        actrail_command(actraild, config, "run"),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    try:
        wait_for_daemon(daemon, args.daemon_ready_timeout_sec)
        if config is None:
            workload_process = subprocess.Popen(
                actrail_command(
                    actrailctl,
                    config,
                    "launch",
                    "--name",
                    "http-payload-unified-e2e",
                    "--",
                    sys.executable,
                    str(workload),
                ),
                text=True,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            try:
                trace_id, prefix_stdout = read_launch_ready(
                    workload_process,
                    args.workload_timeout_sec,
                )
                release_launched_workload(
                    workload_process,
                    prefix_stdout,
                    args.workload_timeout_sec,
                )
            finally:
                stop_process(workload_process)
        else:
            workload_process = subprocess.Popen(
                [sys.executable, str(workload)],
                text=True,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            try:
                pid = read_agent_pid(workload_process, args.workload_timeout_sec)
                trace_id = attach_trace(actrailctl, config, pid)
                release_workload(workload_process, args.workload_timeout_sec)
            finally:
                stop_process(workload_process)
        _, payloads = wait_for_payload_and_events(
            actrailctl,
            actrailviewer,
            config,
            trace_id,
            args.drain_attempts,
            args.drain_sleep_sec,
            args.events_head,
            args.payload_head,
        )
        text = payload_texts(actrailviewer, config, trace_id, payloads)
        for expected in [
            "POST /plain-http HTTP/1.1",
            "Host: local.actrail",
            '"source":"actrail-http"',
            "HTTP/1.1 200 OK",
            "actrail-http-ok",
        ]:
            if expected not in text:
                raise RuntimeError(f"payload text missed expected value: {expected}")
        if "actrail-non-http" in text:
            raise RuntimeError("non-HTTP socket bytes were persisted as payload")
    finally:
        stop_process(daemon)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"HTTP payload e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
