#!/usr/bin/env python3
"""End-to-end coverage for the built-in OTEL JSONL observation plugin path."""

from __future__ import annotations

import json
import os
import shutil
import signal
import sqlite3
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
RUN_DIR = Path("/tmp/actrail-plugin-otel-jsonl")
INSTANCE = "builtin.otel-jsonl"
MARKER = "ACTRAIL_PLUGIN_OTEL_JSONL_E2E"


def run(
    cmd: list[str],
    *,
    timeout: int = 60,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        cmd,
        cwd=ROOT,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=timeout,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"command failed {cmd}: exit={completed.returncode}\n"
            f"{completed.stdout[-4000:]}"
        )
    return completed


def wait_for_socket(path: Path, timeout: float = 10.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.exists():
            return
        time.sleep(0.05)
    raise RuntimeError(f"daemon socket did not appear: {path}")


def read_jsonl(path: Path) -> list[dict]:
    if not path.exists():
        raise RuntimeError(f"live OTEL JSONL file missing: {path}")
    records = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.strip():
            records.append(json.loads(line))
    if not records:
        raise RuntimeError(f"live OTEL JSONL file is empty: {path}")
    return records


def document_contains(document: object, marker: str) -> bool:
    return marker in json.dumps(document, sort_keys=True)


def parse_status_fields(raw: str) -> dict[str, str]:
    fields: dict[str, str] = {}
    for line in raw.splitlines():
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        fields[key.strip()] = value.strip()
    return fields


def status_int(status: dict[str, str], key: str) -> int:
    if key not in status:
        raise RuntimeError(f"plugin status missed {key}\n{status}")
    try:
        return int(status[key])
    except ValueError as error:
        raise RuntimeError(f"plugin status {key} is not an integer\n{status}") from error


def plugin_status(actraild: Path, config: Path) -> dict[str, str]:
    return parse_status_fields(
        run(
            [
                str(actraild),
                "--config",
                str(config),
                "plugin",
                "status",
                "--instance",
                INSTANCE,
            ]
        ).stdout
    )


def assert_builtin_plugin_active(actraild: Path, config: Path) -> None:
    status = plugin_status(actraild, config)
    expected = {
        "instance": INSTANCE,
        "plugin_id": "otel-jsonl",
        "purpose": "observation-consumer",
        "runtime": "builtin",
        "state": "active",
    }
    for key, value in expected.items():
        if status.get(key) != value:
            raise RuntimeError(f"builtin OTEL plugin status {key} != {value}\n{status}")


def assert_builtin_plugin_observed(actraild: Path, config: Path) -> int:
    status = plugin_status(actraild, config)
    observed = status_int(status, "observed_records")
    if observed <= 0:
        raise RuntimeError(f"builtin OTEL plugin did not observe records\n{status}")
    if status.get("last_error", "") not in ("", "none"):
        raise RuntimeError(f"builtin OTEL plugin reported last_error\n{status}")
    return observed


def main() -> int:
    bin_dir = Path(os.environ.get("ACTRAIL_BIN_DIR", ROOT / "target/release"))
    actraild = bin_dir / "actraild"
    actrailctl = bin_dir / "actrailctl"
    actrailviewer = bin_dir / "actrailviewer"
    config = Path(__file__).with_name("operator.conf")
    socket_path = RUN_DIR / "actraild.sock"
    jsonl_path = RUN_DIR / "live-spans.otlp.jsonl"
    db_path = RUN_DIR / "actrail.sqlite"
    export_path = RUN_DIR / "exported.otlp.json"

    if RUN_DIR.exists():
        shutil.rmtree(RUN_DIR)
    RUN_DIR.mkdir(parents=True)

    daemon = subprocess.Popen(
        [str(actraild), "--config", str(config), "run"],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    try:
        wait_for_socket(socket_path)
        assert_builtin_plugin_active(actraild, config)
        run(
            [
                str(actrailctl),
                "--config",
                str(config),
                "--socket-path",
                str(socket_path),
                "launch",
                "--name",
                MARKER,
                "--",
                "bash",
                "-lc",
                f"echo {MARKER}; cat /etc/hostname >/dev/null",
            ],
            timeout=60,
            env=os.environ.copy(),
        )
        time.sleep(1.0)

        spans = read_jsonl(jsonl_path)
        if not any(document_contains(span, MARKER) for span in spans):
            raise RuntimeError("live OTEL JSONL spans do not contain the workload marker")

        with sqlite3.connect(db_path) as conn:
            semantic_actions = conn.execute("select count(*) from semantic_actions").fetchone()[0]
        if semantic_actions <= 0:
            raise RuntimeError("SQLite has no semantic actions")
        observed = assert_builtin_plugin_observed(actraild, config)

        run(
            [
                str(actrailviewer),
                "--config",
                str(config),
                "export-otel",
                "--trace-id",
                "trace-1",
                "--output",
                str(export_path),
            ],
            timeout=60,
        )
        exported = json.loads(export_path.read_text(encoding="utf-8"))
        if not document_contains(exported, MARKER):
            raise RuntimeError("viewer OTEL export does not contain the workload marker")

        print(f"plugin_otel_jsonl_spans={len(spans)}")
        print(f"plugin_otel_jsonl_observed_records={observed}")
        print(f"plugin_otel_jsonl_semantic_actions={semantic_actions}")
        print(f"plugin_otel_jsonl_marker={MARKER}")
        print(f"plugin_otel_jsonl_output={jsonl_path}")
        return 0
    finally:
        if daemon.poll() is None:
            daemon.send_signal(signal.SIGINT)
            try:
                daemon.wait(timeout=10)
            except subprocess.TimeoutExpired:
                daemon.kill()
                daemon.wait(timeout=5)
        if daemon.returncode not in (0, -signal.SIGINT):
            output = daemon.stdout.read() if daemon.stdout else ""
            raise RuntimeError(f"daemon exited unexpectedly: {daemon.returncode}\n{output[-4000:]}")


if __name__ == "__main__":
    raise SystemExit(main())
