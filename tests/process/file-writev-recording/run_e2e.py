#!/usr/bin/env python3
"""Verify writev to a regular file is projected as file.write."""

from __future__ import annotations

import argparse
import os
import re
import select
import shutil
import signal
import sqlite3
import subprocess
import sys
import time
from pathlib import Path


TRACE_RE = re.compile(r"trace trace-(\d+) entered Active")
EXPECTED_BYTES = len(b"alpha-beta\n") + len(b"gamma-delta\n")


def parse_args() -> argparse.Namespace:
    test_dir = Path(__file__).resolve().parent
    template = test_dir.parent / "file-scan-recording" / "operator.conf"
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--template-config", default=str(template))
    parser.add_argument("--ready-timeout-sec", type=float, default=30.0)
    parser.add_argument("--completion-timeout-sec", type=float, default=60.0)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    repo = Path(__file__).resolve().parents[3]
    bin_dir = (repo / args.bin_dir).resolve()
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    require_tool("python3")
    config = Path("/tmp/actrail-file-writev-recording.conf")
    storage = Path("/tmp/actrail-file-writev-recording.sqlite")
    output_path = Path("/tmp/actrail-file-writev-recording-output.txt")
    clean_paths(config, output_path)
    write_config(Path(args.template_config), config)
    daemon = subprocess.Popen(
        [str(actraild), "--config", str(config), "run"],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    try:
        wait_for_daemon(daemon, args.ready_timeout_sec)
        trace_id, output = run_writev_workload(
            actrailctl, config, output_path, args.ready_timeout_sec
        )
        wait_for_clean_trace(storage, trace_id, args.completion_timeout_sec)
        verify_writev_recording(storage, trace_id, output_path)
        print(f"file writev recording e2e passed trace=trace-{trace_id}")
        print(output, end="")
        return 0
    finally:
        stop_daemon(daemon)
        print_daemon_stderr(daemon)
        clean_paths(config, output_path)


def require_binary(bin_dir: Path, name: str) -> Path:
    path = bin_dir / name
    if not path.exists():
        raise RuntimeError(f"missing binary {path}; build with cargo build --release")
    return path


def require_tool(name: str) -> None:
    if shutil.which(name) is None:
        raise RuntimeError(f"missing required tool {name}")


def write_config(template: Path, target: Path) -> None:
    raw = template.read_text(encoding="utf-8")
    raw = raw.replace("file-scan-recording", "file-writev-recording")
    raw = raw.replace("127.0.0.1:18082", "127.0.0.1:18083")
    target.write_text(raw, encoding="utf-8")


def clean_paths(config: Path, output_path: Path) -> None:
    for path in [
        config,
        output_path,
        Path("/tmp/actrail-file-writev-recording.sock"),
        Path("/tmp/actrail-file-writev-recording.pid"),
        Path("/tmp/actrail-file-writev-recording.sqlite"),
        Path("/tmp/actrail-file-writev-recording.log"),
        Path("/tmp/actrail-file-writev-recording-tls-sync.sock"),
    ]:
        if path.exists():
            path.unlink()
    export_dir = Path("/tmp/actrail-file-writev-recording-export")
    if export_dir.exists():
        shutil.rmtree(export_dir)


def wait_for_daemon(process: subprocess.Popen[str], timeout_sec: float) -> None:
    deadline = time.monotonic() + timeout_sec
    assert process.stdout is not None
    while time.monotonic() < deadline:
        line = read_line(process, deadline)
        if line:
            print(line, end="")
            if "daemon listening" in line:
                return
        if process.poll() is not None:
            stderr = process.stderr.read() if process.stderr else ""
            raise RuntimeError(f"actraild exited early: {stderr}")
    raise RuntimeError("actraild did not become ready")


def read_line(process: subprocess.Popen[str], deadline: float) -> str:
    assert process.stdout is not None
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        return ""
    readable, _, _ = select.select([process.stdout], [], [], remaining)
    if readable:
        return process.stdout.readline()
    return ""


def run_writev_workload(
    actrailctl: Path,
    config: Path,
    output_path: Path,
    timeout_sec: float,
) -> tuple[int, str]:
    script = (
        "import os, time; "
        f"path = {str(output_path)!r}; "
        "fd = os.open(path, os.O_CREAT | os.O_WRONLY, 0o600); "
        "os.writev(fd, [b'alpha-', b'beta\\n']); "
        "time.sleep(0.05); "
        "os.writev(fd, [b'gamma-', b'delta\\n']); "
        "os.close(fd)"
    )
    process = subprocess.Popen(
        [
            str(actrailctl),
            "--config",
            str(config),
            "launch",
            "--name",
            "file-writev-recording",
            "--",
            "python3",
            "-c",
            script,
        ],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    try:
        stdout, stderr = process.communicate(timeout=timeout_sec)
    except subprocess.TimeoutExpired as error:
        process.kill()
        stdout, stderr = process.communicate()
        raise RuntimeError(f"writev workload timed out stdout={stdout} stderr={stderr}") from error
    if process.returncode != 0:
        raise RuntimeError(
            f"writev workload failed exit={process.returncode} stdout={stdout} stderr={stderr}"
        )
    print(stdout, end="")
    match = TRACE_RE.search(stdout)
    if match is None:
        raise RuntimeError(f"trace id missing from actrailctl output: {stdout}")
    return int(match.group(1)), stdout


def wait_for_clean_trace(storage: Path, trace_id: int, timeout_sec: float) -> None:
    deadline = time.monotonic() + timeout_sec
    while time.monotonic() < deadline:
        if storage.exists():
            with sqlite3.connect(storage) as connection:
                row = connection.execute(
                    "SELECT lifecycle_state, health FROM traces WHERE trace_id = ?",
                    (trace_id,),
                ).fetchone()
                if row in {("completed", "clean"), ("exited", "clean")}:
                    return
                if row and row[0] == "failed":
                    raise RuntimeError(f"trace-{trace_id} failed health={row[1]}")
        time.sleep(0.2)
    raise RuntimeError(f"trace-{trace_id} did not complete cleanly")


def verify_writev_recording(storage: Path, trace_id: int, output_path: Path) -> None:
    if output_path.read_bytes() != b"alpha-beta\ngamma-delta\n":
        raise RuntimeError(f"unexpected writev output in {output_path}")
    with sqlite3.connect(storage) as connection:
        writev_raw_count = connection.execute(
            """
            SELECT COUNT(*)
            FROM events
            WHERE trace_id = ?
              AND payload_variant = 'file'
              AND payload_fields LIKE '%operation=writev%'
              AND payload_fields LIKE ?
            """,
            (trace_id, f"%path={str(output_path)}%"),
        ).fetchone()[0]
        if writev_raw_count < 2:
            raise RuntimeError(f"expected at least two raw file writev events, got {writev_raw_count}")
        rows = connection.execute(
            """
            SELECT attributes
            FROM semantic_actions
            WHERE trace_id = ? AND kind = 'file.write'
            """,
            (trace_id,),
        ).fetchall()
        bytes_written = 0
        write_actions = 0
        for (raw_attributes,) in rows:
            attributes = decode_map(raw_attributes)
            if attributes.get("file.path") != str(output_path):
                continue
            write_actions += 1
            bytes_written += int(attributes.get("file.bytes_written", "0"))
        if write_actions < 1 or bytes_written < EXPECTED_BYTES:
            raise RuntimeError(
                "missing file.write action bytes for regular-file writev: "
                f"actions={write_actions} bytes={bytes_written}"
            )


def decode_map(raw: str) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in raw.splitlines():
        key, separator, value = line.partition("=")
        if not separator:
            continue
        values[decode_escaped(key)] = decode_escaped(value)
    return values


def decode_escaped(raw: str) -> str:
    output: list[str] = []
    index = 0
    while index < len(raw):
        char = raw[index]
        if char == "\\" and index + 1 < len(raw):
            escaped = raw[index + 1]
            if escaped == "n":
                output.append("\n")
                index += 2
                continue
            if escaped == "e":
                output.append("=")
                index += 2
                continue
            if escaped == "\\":
                output.append("\\")
                index += 2
                continue
        output.append(char)
        index += 1
    return "".join(output)


def stop_daemon(process: subprocess.Popen[str]) -> None:
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)


def print_daemon_stderr(process: subprocess.Popen[str]) -> None:
    if process.stderr is None:
        return
    stderr = process.stderr.read()
    if stderr:
        print(stderr, end="", file=sys.stderr)


if __name__ == "__main__":
    raise SystemExit(main())
