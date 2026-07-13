#!/usr/bin/env python3
"""Verify repeated file scans reuse canonical path sets."""

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

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from action_snapshot import ActionRecord, SemanticActionSnapshot  # noqa: E402


TRACE_RE = re.compile(r"trace trace-(\d+) entered Active")
FAST_PATH_READ_COUNT_ATTR = "file.bulk_read.fp_read_count"
FAST_PATH_SUMMARY_COUNT_ATTR = "file.bulk_read.fp_summary_count"


def parse_args() -> argparse.Namespace:
    test_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--config", default=str(test_dir / "operator.conf"))
    parser.add_argument("--ready-timeout-sec", type=float, default=30.0)
    parser.add_argument("--completion-timeout-sec", type=float, default=60.0)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    repo = Path(__file__).resolve().parents[3]
    bin_dir = (repo / args.bin_dir).resolve()
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
    require_tool("rg")
    config = Path(args.config).resolve()
    values = read_config(config)
    storage = Path(values["storage_sqlite_path"])
    scan_dir = Path("/tmp/actrail-file-scan-recording-tree")
    clean_paths(values, scan_dir)
    create_scan_tree(scan_dir)
    daemon = subprocess.Popen(
        [str(actraild), "--config", str(config), "run"],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    try:
        wait_for_daemon(daemon, args.ready_timeout_sec)
        trace_id, output = run_scan_workload(actrailctl, config, scan_dir, args.ready_timeout_sec)
        wait_for_completed_trace(storage, trace_id, args.completion_timeout_sec)
        verify_scan_recording(storage, actrailviewer, config, trace_id)
        print(f"file scan recording e2e passed trace=trace-{trace_id}")
        print(output, end="")
        return 0
    finally:
        stop_daemon(daemon)
        print_daemon_stderr(daemon)
        if scan_dir.exists():
            shutil.rmtree(scan_dir)


def require_binary(bin_dir: Path, name: str) -> Path:
    path = bin_dir / name
    if not path.exists():
        raise RuntimeError(f"missing binary {path}; build with cargo build --release")
    return path


def require_tool(name: str) -> None:
    if shutil.which(name) is None:
        raise RuntimeError(f"missing required tool {name}")


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
            continue
        key = key.strip()
        value = value.strip().strip('"')
        remapped = operator_config_key(section, key)
        if remapped is not None:
            values.setdefault(remapped, value)
    return values


def operator_config_key(section: str, key: str) -> str | None:
    if section == "control" and key in {"socket_path", "pid_file", "log_path"}:
        return key
    if section == "storage.sqlite" and key == "path":
        return "storage_sqlite_path"
    if section == "export.snapshot" and key == "directory":
        return "export_directory"
    if section == "export.runtime.routes.otel_jsonl" and key == "path":
        return "export_otel_jsonl_path"
    if section == "payload.tls" and key == "sync_event_socket_path":
        return "payload_tls_sync_event_socket_path"
    if not section:
        return key
    return None


def clean_paths(values: dict[str, str], scan_dir: Path) -> None:
    for key in [
        "socket_path",
        "pid_file",
        "storage_sqlite_path",
        "log_path",
        "payload_tls_sync_event_socket_path",
        "export_otel_jsonl_path",
    ]:
        path = values.get(key)
        if path and Path(path).exists():
            Path(path).unlink()
    export_dir = values.get("export_directory")
    if export_dir and Path(export_dir).exists():
        shutil.rmtree(export_dir)
    if scan_dir.exists():
        shutil.rmtree(scan_dir)


def create_scan_tree(scan_dir: Path) -> None:
    scan_dir.mkdir(parents=True)
    for index in range(12):
        subdir = scan_dir / f"dir-{index % 3}"
        subdir.mkdir(exist_ok=True)
        path = subdir / f"file-{index}.txt"
        path.write_text(
            "\n".join(
                [
                    f"ACTRAIL_SCAN_ALPHA line {index}",
                    f"ACTRAIL_SCAN_BETA line {index}",
                    "ordinary text",
                ]
            )
            + "\n",
            encoding="utf-8",
        )


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


def run_scan_workload(
    actrailctl: Path,
    config: Path,
    scan_dir: Path,
    timeout_sec: float,
) -> tuple[int, str]:
    script = (
        f"rg --no-mmap ACTRAIL_SCAN_NO_MATCH_A {scan_dir} >/dev/null || true; "
        f"rg --no-mmap ACTRAIL_SCAN_NO_MATCH_B {scan_dir} >/dev/null || true"
    )
    process = subprocess.Popen(
        [
            str(actrailctl),
            "--config",
            str(config),
            "launch",
            "--name",
            "file-scan-recording",
            "--",
            "sh",
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
        raise RuntimeError(f"scan workload timed out stdout={stdout} stderr={stderr}") from error
    if process.returncode != 0:
        raise RuntimeError(
            f"scan workload failed exit={process.returncode} stdout={stdout} stderr={stderr}"
        )
    print(stdout, end="")
    match = TRACE_RE.search(stdout)
    if match is None:
        raise RuntimeError(f"trace id missing from actrailctl output: {stdout}")
    return int(match.group(1)), stdout


def wait_for_completed_trace(storage: Path, trace_id: int, timeout_sec: float) -> None:
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


def verify_scan_recording(
    storage: Path,
    actrailviewer: Path,
    config: Path,
    trace_id: int,
) -> None:
    snapshot = SemanticActionSnapshot.load(actrailviewer, config, trace_id)
    bulk_actions = snapshot.actions("file.bulk_read")
    if len(bulk_actions) < 2:
        raise RuntimeError(
            f"expected at least two file.bulk_read actions, got {len(bulk_actions)}"
        )
    with sqlite3.connect(storage) as connection:
        verify_fast_path_bulk_read(bulk_actions)
        verify_no_event_transport_loss(connection, trace_id)
        action_ids = tuple(action.action_id for action in bulk_actions)
        placeholders = ",".join("?" for _ in action_ids)
        reused_path_set_count = scalar(
            connection,
            f"""
            SELECT COUNT(*)
            FROM (
                SELECT refs.path_set_id
                FROM file_path_set_action_refs refs
                JOIN semantic_action_ids ids
                  ON ids.action_key = refs.action_key
                WHERE refs.trace_id = ?
                  AND ids.action_id IN ({placeholders})
                GROUP BY refs.path_set_id
                HAVING COUNT(*) >= 2
            )
            """,
            (trace_id, *action_ids),
        )
        if reused_path_set_count < 1:
            raise RuntimeError("expected repeated bulk reads to share a canonical path set")
        bulk_processes = {action.process_id for action in bulk_actions}
        leaked_read_links = sum(
            child.process_id in bulk_processes
            for child in snapshot.valid_linked_children(
                "command.contains_file_access", "file.read"
            )
        )
        if leaked_read_links != 0:
            raise RuntimeError(
                "aggregated scan process leaked command.contains_file_access -> file.read links: "
                f"{leaked_read_links}"
            )


def verify_fast_path_bulk_read(actions: tuple[ActionRecord, ...]) -> None:
    summary_count = 0
    read_count = 0
    for action in actions:
        summary_count += int(action.attributes.get(FAST_PATH_SUMMARY_COUNT_ATTR, "0"))
        read_count += int(action.attributes.get(FAST_PATH_READ_COUNT_ATTR, "0"))
    if summary_count <= 0 or read_count <= 0:
        raise RuntimeError(
            "expected file.bulk_read actions to include fast-path read_summary counts, "
            f"got summary_count={summary_count} read_count={read_count}"
        )


def verify_no_event_transport_loss(connection: sqlite3.Connection, trace_id: int) -> None:
    loss_count = scalar(
        connection,
        """
        SELECT COUNT(*)
        FROM diagnostics
        WHERE trace_id = ? AND kind = 'event_transport_loss'
        """,
        (trace_id,),
    )
    if loss_count != 0:
        raise RuntimeError(f"expected no event_transport_loss diagnostics, got {loss_count}")


def scalar(connection: sqlite3.Connection, sql: str, params: tuple[object, ...]) -> int:
    row = connection.execute(sql, params).fetchone()
    if row is None:
        raise RuntimeError(f"query returned no rows: {sql}")
    return int(row[0])


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
