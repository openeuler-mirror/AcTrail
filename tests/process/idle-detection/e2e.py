#!/usr/bin/env python3
"""Exercise OpenCode idle reporting from plugin loading through SQLite storage.

The ``opencode`` executable used here is a small local host fixture.  It has
the same process environment that a real OpenCode launch receives, loads the
real injected plugin from ``OPENCODE_CONFIG_DIR``, and sends structured
OpenCode events to it.  Thus the tested path is:

actrailctl launch -> injected plugin -> UDS -> daemon -> IdleDetector -> SQLite
"""

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
PHASE_RE = re.compile(r"^PHASE ([-a-z]+)$")
TEST_NAME = "idle-detection"
TASK_PREFIX = "idle-e2e"


def parse_args() -> argparse.Namespace:
    directory = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument(
        "--template-config",
        default=str(directory.parent / "file-scan-recording" / "operator.conf"),
    )
    parser.add_argument("--ready-timeout-sec", type=float, default=30.0)
    parser.add_argument("--phase-timeout-sec", type=float, default=12.0)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    repo = Path(__file__).resolve().parents[3]
    bin_dir = (repo / args.bin_dir).resolve()
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    require_tool("node")

    config = Path(f"/tmp/actrail-{TEST_NAME}.conf")
    storage = Path(f"/tmp/actrail-{TEST_NAME}.sqlite")
    fixture_bin = Path(f"/tmp/actrail-{TEST_NAME}-bin")
    clean_paths(config, fixture_bin)
    write_config(Path(args.template_config), config, repo)
    opencode = make_opencode_launcher(fixture_bin, Path(__file__).with_name("plugin-host-fixture.mjs"))
    daemon = subprocess.Popen(
        [str(actraild), "--config", str(config), "run"],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    launch: subprocess.Popen[str] | None = None
    try:
        wait_for_daemon(daemon, args.ready_timeout_sec)
        environment = os.environ.copy()
        environment["ACTRAIL_OPENCODE_TASK_ID"] = TASK_PREFIX
        launch = subprocess.Popen(
        [str(actrailctl), "--config", str(config), "launch", "--name", TEST_NAME, "--", str(opencode)],
        text=True,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=environment,
        )
        trace_id = wait_for_trace_id(launch, args.phase_timeout_sec)

        wait_for_phase(launch, "idle-open", args.phase_timeout_sec)
        first = wait_for_intervals(storage, trace_id, 1, args.phase_timeout_sec)
        require(first[0]["end_time"] is None, f"first idle interval must be open: {first}")

        advance_fixture(launch)
        wait_for_phase(launch, "waiting-elapsed", args.phase_timeout_sec)
        waiting = read_intervals(storage, trace_id)
        require(len(waiting) == 1, f"permission wait created a new idle interval: {waiting}")
        require(waiting[0]["end_time"] is not None, f"permission request did not close idle: {waiting}")

        advance_fixture(launch)
        wait_for_phase(launch, "resumed-idle", args.phase_timeout_sec)
        resumed = wait_for_intervals(storage, trace_id, 2, args.phase_timeout_sec)
        require(resumed[-1]["end_time"] is None, f"resumed idle interval must be open: {resumed}")

        advance_fixture(launch)
        wait_for_phase(launch, "completed", args.phase_timeout_sec)
        stdout, stderr = launch.communicate(timeout=args.phase_timeout_sec)
        if launch.returncode != 0:
            raise RuntimeError(f"plugin host failed exit={launch.returncode} stdout={stdout} stderr={stderr}")
        final = wait_for_closed_intervals(storage, trace_id, 2, args.phase_timeout_sec)
        require(all(item["task_id"].startswith(f"{TASK_PREFIX}:session:") for item in final), final)
        print(f"idle detection e2e passed trace=trace-{trace_id}")
        return 0
    finally:
        if launch is not None and launch.poll() is None:
            launch.terminate()
            launch.wait(timeout=5)
        stop_process_group(daemon)
        print_stderr(daemon)
        clean_paths(config, fixture_bin)


def require_binary(directory: Path, name: str) -> Path:
    path = directory / name
    if not path.is_file():
        raise RuntimeError(f"missing binary {path}; build with cargo build --release")
    return path


def require_tool(name: str) -> None:
    if shutil.which(name) is None:
        raise RuntimeError(f"missing required tool {name}")


def write_config(template: Path, destination: Path, repo: Path) -> None:
    raw = template.read_text(encoding="utf-8")
    raw = raw.replace("file-scan-recording", TEST_NAME).replace("127.0.0.1:18082", "127.0.0.1:18084")
    raw = raw.replace(
        'capabilities = [\n  "proc-lifecycle",\n  "proc-exec-context",\n  "fs-access-basic",\n]',
        'capabilities = ["proc-lifecycle"]',
    )
    raw = raw.replace("[ebpf]\nenabled = true", "[ebpf]\nenabled = false")
    raw = raw.replace("[seccomp_notify]\nenabled = true", "[seccomp_notify]\nenabled = false")
    raw = raw.replace("[process_seccomp]\nenabled = true", "[process_seccomp]\nenabled = false")
    # The shared file-scan template intentionally keeps shutdown quick for its
    # own workload.  Current daemon validation requires the full finalization
    # budget here, so make the E2E's derived, isolated config self-consistent.
    raw = raw.replace("shutdown_wait_ms = 5000", "shutdown_wait_ms = 150100")
    if "shutdown_wait_ms = 150100" not in raw:
        raise RuntimeError("template config has no compatible supervision.shutdown_wait_ms")
    raw += (
        "\n[idle_detection]\n"
        "enabled = true\n"
        "threshold_secs = \"1s\"\n"
        "opencode_auto_inject = true\n"
        f'opencode_plugin_dir = "{repo / "deploy/agent-host/opencode"}"\n'
        "\n[alert_forwarding]\n"
        # The daemon initializes alert forwarding even though this scenario
        # emits no alerts. Use a checked-in disabled plugin configuration,
        # rather than a system-wide production path that CI may not read.
        'proxy_executable = "/usr/bin/true"\n'
        f'proxy_config_path = "{destination}"\n'
        f'plugin_config_path = "{repo / "examples/plugins/builtin/alert-forwarding/alert-forwarding.config.json"}"\n'
        f'socket_path = "/tmp/actrail-{TEST_NAME}-alert-proxy.sock"\n'
    )
    destination.write_text(raw, encoding="utf-8")


def make_opencode_launcher(directory: Path, fixture: Path) -> Path:
    directory.mkdir(mode=0o700)
    launcher = directory / "opencode"
    launcher.write_text(f"#!/bin/sh\nexec node {fixture!s}\n", encoding="utf-8")
    launcher.chmod(0o700)
    return launcher


def clean_paths(config: Path, fixture_bin: Path) -> None:
    for path in [
        config,
        Path(f"/tmp/actrail-{TEST_NAME}.sock"),
        Path(f"/tmp/actrail-{TEST_NAME}.pid"),
        Path(f"/tmp/actrail-{TEST_NAME}.sqlite"),
        Path(f"/tmp/actrail-{TEST_NAME}.log"),
        Path(f"/tmp/actrail-{TEST_NAME}-tls-sync.sock"),
    ]:
        if path.exists():
            path.unlink()
    if fixture_bin.exists():
        shutil.rmtree(fixture_bin)
    export = Path(f"/tmp/actrail-{TEST_NAME}-export")
    if export.exists():
        shutil.rmtree(export)


def wait_for_daemon(process: subprocess.Popen[str], timeout: float) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        line = read_line(process, deadline)
        if "daemon listening" in line:
            return
        if process.poll() is not None:
            raise RuntimeError(f"actraild exited early: {process.stderr.read() if process.stderr else ''}")
    raise RuntimeError("actraild did not become ready")


def wait_for_trace_id(process: subprocess.Popen[str], timeout: float) -> int:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        line = read_line(process, deadline)
        match = TRACE_RE.search(line)
        if match:
            return int(match.group(1))
        if process.poll() is not None:
            raise RuntimeError(f"launch ended before trace creation: {drain_process(process)}")
    raise RuntimeError("trace id missing from actrailctl output")


def wait_for_phase(process: subprocess.Popen[str], expected: str, timeout: float) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        line = read_line(process, deadline).strip()
        match = PHASE_RE.match(line)
        if match and match.group(1) == expected:
            return
        if process.poll() is not None:
            raise RuntimeError(f"plugin host ended before phase {expected}: {drain_process(process)}")
    raise RuntimeError(f"timed out waiting for phase {expected}")


def advance_fixture(process: subprocess.Popen[str]) -> None:
    assert process.stdin is not None
    process.stdin.write("continue\n")
    process.stdin.flush()


def read_line(process: subprocess.Popen[str], deadline: float) -> str:
    assert process.stdout is not None
    readable, _, _ = select.select([process.stdout], [], [], max(0.0, deadline - time.monotonic()))
    return process.stdout.readline() if readable else ""


def drain_process(process: subprocess.Popen[str]) -> str:
    stdout, stderr = process.communicate()
    return f"stdout={stdout} stderr={stderr}"


def read_intervals(storage: Path, trace_id: int) -> list[dict[str, object]]:
    if not storage.exists():
        return []
    with sqlite3.connect(storage) as connection:
        connection.row_factory = sqlite3.Row
        rows = connection.execute(
            """
            SELECT task_id, start_time, end_time
            FROM idle_intervals WHERE trace_id = ? ORDER BY interval_id
            """,
            (trace_id,),
        ).fetchall()
    return [dict(row) for row in rows]


def wait_for_intervals(storage: Path, trace_id: int, count: int, timeout: float) -> list[dict[str, object]]:
    return wait_until(
        lambda: rows if len(rows := read_intervals(storage, trace_id)) >= count else None,
        timeout,
    )


def wait_for_closed_intervals(storage: Path, trace_id: int, count: int, timeout: float) -> list[dict[str, object]]:
    return wait_until(
        lambda: rows
        if len(rows := read_intervals(storage, trace_id)) >= count
        and all(row["end_time"] is not None for row in rows)
        else None,
        timeout,
    )


def wait_until(predicate, timeout: float):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.1)
    raise RuntimeError("timed out waiting for expected SQLite state")


def require(condition: bool, message: object) -> None:
    if not condition:
        raise RuntimeError(str(message))


def stop_process_group(process: subprocess.Popen[str]) -> None:
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)


def print_stderr(process: subprocess.Popen[str]) -> None:
    if process.stderr:
        stderr = process.stderr.read()
        if stderr:
            print(stderr, end="", file=sys.stderr)


if __name__ == "__main__":
    raise SystemExit(main())
