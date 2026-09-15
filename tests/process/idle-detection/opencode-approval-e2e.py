#!/usr/bin/env python3
"""Verify a real OpenCode CLI approval suppresses idle intervals.

This scenario deliberately uses the installed OpenCode CLI and model rather
than the deterministic plugin host used by e2e.py. It starts OpenCode via
actrailctl, asks it to execute ``pwd``, and keeps its required approval pending
past the configured idle threshold. No idle interval may be recorded while the
approval is pending.
"""

from __future__ import annotations

import argparse
import os
import shutil
import sqlite3
import subprocess
import time
from pathlib import Path

from e2e import (
    TEST_NAME,
    clean_paths,
    print_stderr,
    require_binary,
    require_tool,
    stop_process_group,
    wait_for_daemon,
    wait_for_trace_id,
    write_config,
)


def parse_args() -> argparse.Namespace:
    directory = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument(
        "--template-config",
        default=str(directory.parent / "file-scan-recording" / "operator.conf"),
    )
    parser.add_argument("--opencode-bin", default="opencode")
    parser.add_argument("--model", default="opencode/big-pickle")
    parser.add_argument("--ready-timeout-sec", type=float, default=30.0)
    parser.add_argument("--pending-observation-sec", type=float, default=2.0)
    parser.add_argument(
        "--prompt",
        default="必须立即使用 bash 工具执行 pwd；不要解释、不要提问、不要使用其他工具。",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    repo = Path(__file__).resolve().parents[3]
    bin_dir = (repo / args.bin_dir).resolve()
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    require_tool("node")
    opencode = shutil.which(args.opencode_bin)
    if opencode is None:
        raise RuntimeError(f"missing OpenCode executable: {args.opencode_bin}")

    config = Path(f"/tmp/actrail-{TEST_NAME}-real-opencode.conf")
    storage = Path(f"/tmp/actrail-{TEST_NAME}-real-opencode.sqlite")
    fixture_bin = Path(f"/tmp/actrail-{TEST_NAME}-real-opencode-bin")
    project = Path(f"/tmp/actrail-{TEST_NAME}-real-opencode-project")
    clean_paths(config, fixture_bin)
    clean_real_paths(storage, project)
    write_config(Path(args.template_config), config, repo)
    isolate_runtime_paths(config)
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
        project.mkdir(mode=0o700)
        (project / "opencode.json").write_text(
            '{"permission":{"*":"ask","bash":"ask"}}\n', encoding="utf-8"
        )
        launch = subprocess.Popen(
            [
                str(actrailctl), "--config", str(config), "launch",
                "--name", "opencode-approval-e2e", "--", opencode, "run",
                "--dir", str(project), "--model", args.model, args.prompt,
            ],
            cwd=project,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        trace_id = wait_for_trace_id(launch, args.ready_timeout_sec)
        assert_no_idle_intervals_while_pending(
            storage, trace_id, launch, args.pending_observation_sec
        )
        print(f"OpenCode approval E2E passed trace=trace-{trace_id}")
        return 0
    finally:
        if launch is not None and launch.poll() is None:
            launch.terminate()
            launch.wait(timeout=10)
        stop_process_group(daemon)
        print_stderr(daemon)
        clean_paths(config, fixture_bin)
        clean_real_paths(storage, project)


def assert_no_idle_intervals_while_pending(
    storage: Path,
    trace_id: int,
    launch: subprocess.Popen[str],
    observation_sec: float,
) -> None:
    deadline = time.monotonic() + observation_sec
    while time.monotonic() < deadline:
        if launch.poll() is not None:
            stdout, stderr = launch.communicate()
            raise RuntimeError(
                "OpenCode exited before the pending-approval observation window: "
                f"exit={launch.returncode} stdout={stdout} stderr={stderr}"
            )
        with sqlite3.connect(storage) as connection:
            count = connection.execute(
                "SELECT COUNT(*) FROM idle_intervals WHERE trace_id = ?",
                (trace_id,),
            ).fetchone()[0]
        if count:
            raise RuntimeError(
                "pending OpenCode approval created an idle interval: "
                f"trace=trace-{trace_id} count={count}"
            )
        time.sleep(0.1)


def clean_real_paths(storage: Path, project: Path) -> None:
    for path in [
        storage,
        Path(f"/tmp/actrail-{TEST_NAME}-real-opencode.sock"),
        Path(f"/tmp/actrail-{TEST_NAME}-real-opencode.pid"),
        Path(f"/tmp/actrail-{TEST_NAME}-real-opencode.log"),
        Path(f"/tmp/actrail-{TEST_NAME}-real-opencode-tls-sync.sock"),
    ]:
        if path.exists():
            path.unlink()
    if project.exists():
        shutil.rmtree(project)


def isolate_runtime_paths(config: Path) -> None:
    raw = config.read_text(encoding="utf-8")
    raw = raw.replace(f"actrail-{TEST_NAME}", f"actrail-{TEST_NAME}-real-opencode")
    config.write_text(raw, encoding="utf-8")


if __name__ == "__main__":
    raise SystemExit(main())
