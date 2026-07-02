#!/usr/bin/env python3
"""Run eBPF event-transport loss containment E2E."""

from __future__ import annotations

import argparse
import os
import select
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
SOURCE_CONFIG = ROOT / "docs/examples/01.quick-start/operator.conf"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--daemon-ready-timeout-sec", type=float, default=10.0)
    parser.add_argument("--drain-attempts", type=int, default=20)
    parser.add_argument("--drain-sleep-sec", type=float, default=0.1)
    parser.add_argument("--exec-count", type=int, default=5000)
    return parser.parse_args()


def require_root() -> None:
    if os.geteuid() != 0:
        raise RuntimeError("eBPF transport-loss E2E requires root for eBPF attach")


def require_binary(bin_dir: Path, name: str) -> Path:
    path = bin_dir / name
    if not path.exists():
        raise RuntimeError(f"missing binary {path}; build with cargo build --release")
    return path


def render_config(tmp: Path) -> str:
    raw = SOURCE_CONFIG.read_text(encoding="utf-8")
    raw = raw.replace("/tmp/actrail", str(tmp / "actrail"))
    raw = raw.replace("/tmp/actraild", str(tmp / "actraild"))
    raw = raw.replace("event_ring_buffer_max_bytes = 1048576", "event_ring_buffer_max_bytes = 4096")
    raw = raw.replace("tracked_process_max_entries = 4096", "tracked_process_max_entries = 8192")
    raw = raw.replace("pending_operation_max_entries = 4096", "pending_operation_max_entries = 8192")
    raw = raw.replace('diagnostic_log_level = "info"', 'diagnostic_log_level = "debug"')
    return raw


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


def run_checked(command: list[str], *, timeout: float | None = None) -> str:
    result = subprocess.run(
        command,
        text=True,
        capture_output=True,
        check=False,
        timeout=timeout,
    )
    if result.stdout:
        print(result.stdout, end="")
    if result.stderr:
        print(result.stderr, end="", file=sys.stderr)
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed: {' '.join(command)}\nstdout={result.stdout}\nstderr={result.stderr}"
        )
    return result.stdout


def stop_process(process: subprocess.Popen[str]) -> None:
    if process.poll() is not None:
        return
    process.send_signal(signal.SIGTERM)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def wait_for_transport_loss(
    actrailctl: Path,
    config: Path,
    storage_path: Path,
    attempts: int,
    sleep_sec: float,
) -> tuple[str, str, int]:
    last_rows: list[tuple[str, str, str]] = []
    for _ in range(attempts):
        run_checked([str(actrailctl), "--config", str(config), "list-traces"])
        with sqlite3.connect(storage_path) as connection:
            trace = connection.execute(
                "select lifecycle_state, health from traces where trace_id = 1"
            ).fetchone()
            rows = connection.execute(
                "select kind, message, metadata from diagnostics where trace_id = 1"
            ).fetchall()
        last_rows = rows
        if trace and any("event_transport_loss" in metadata for _, _, metadata in rows):
            return trace[0], trace[1], len(rows)
        time.sleep(sleep_sec)
    raise RuntimeError(f"missing event_transport_loss diagnostic; rows={last_rows}")


def main() -> int:
    args = parse_args()
    require_root()
    bin_dir = ROOT / args.bin_dir
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")

    with tempfile.TemporaryDirectory(prefix="actrail-ebpf-transport-loss-") as raw_tmp:
        tmp = Path(raw_tmp)
        config = tmp / "operator.conf"
        storage_path = tmp / "actrail.sqlite"
        config.write_text(render_config(tmp), encoding="utf-8")
        (tmp / "actrail-enforcement-rules.conf").write_text("", encoding="utf-8")

        daemon = subprocess.Popen(
            [str(actraild), "--config", str(config), "run"],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        try:
            wait_for_daemon(daemon, args.daemon_ready_timeout_sec)
            workload = (
                "i=0; "
                f"while [ $i -lt {args.exec_count} ]; do /bin/true; i=$((i+1)); done"
            )
            run_checked(
                [
                    str(actrailctl),
                    "--config",
                    str(config),
                    "launch",
                    "--seccomp-mode",
                    "skip",
                    "--name",
                    "ebpf-transport-loss-e2e",
                    "--",
                    "/bin/sh",
                    "-c",
                    workload,
                ],
                timeout=30,
            )
            lifecycle_state, health, diagnostic_count = wait_for_transport_loss(
                actrailctl,
                config,
                storage_path,
                args.drain_attempts,
                args.drain_sleep_sec,
            )
            run_checked([str(actrailctl), "--config", str(config), "list-traces"])
            if lifecycle_state not in {"completed", "exited"}:
                raise RuntimeError(f"trace did not reach terminal state: {lifecycle_state}")
            if health != "degraded":
                raise RuntimeError(f"trace was not degraded after transport loss: {health}")
            if daemon.poll() is not None:
                stderr = daemon.stderr.read() if daemon.stderr else ""
                raise RuntimeError(f"daemon exited after transport loss: {stderr}")
            print(
                "ebpf_transport_loss_e2e=ok "
                f"state={lifecycle_state} health={health} diagnostics={diagnostic_count}"
            )
        finally:
            stop_process(daemon)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
