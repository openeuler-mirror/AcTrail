"""Daemon and launch process helpers."""

from __future__ import annotations

import os
import re
import select
import signal
import subprocess
import sys
import time
from pathlib import Path

from .config import actrail_command, run_checked

TRACE_RE = re.compile(r"trace trace-(\d+) entered Active")


def start_daemon(actraild: Path, config: Path | None, timeout_sec: float) -> subprocess.Popen[str]:
    daemon = subprocess.Popen(
        actrail_command(actraild, config, "run"),
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
    config: Path | None,
    name: str,
    argv: list[str],
    timeout_sec: float,
) -> tuple[int, str]:
    command = actrail_command(
        actrailctl,
        config,
        "launch",
        "--name",
        name,
        "--",
        *argv,
    )
    output = run_checked(command, timeout=timeout_sec)
    match = TRACE_RE.search(output)
    if not match:
        raise RuntimeError(f"could not parse trace id from launch output: {output}")
    return int(match.group(1)), output


def launch_and_parse_trace_with_daemon(
    daemon: subprocess.Popen[str],
    actrailctl: Path,
    config: Path | None,
    name: str,
    argv: list[str],
    timeout_sec: float,
    poll_interval_sec: float,
    stop_timeout_sec: float,
) -> tuple[int, str]:
    require_positive_seconds(timeout_sec, "launch timeout")
    require_positive_seconds(poll_interval_sec, "launch poll interval")
    require_positive_seconds(stop_timeout_sec, "launch stop timeout")
    command = actrail_command(
        actrailctl,
        config,
        "launch",
        "--name",
        name,
        "--",
        *argv,
    )
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
