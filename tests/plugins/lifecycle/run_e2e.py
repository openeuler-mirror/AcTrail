#!/usr/bin/env python3
from __future__ import annotations

import os
import shutil
import signal
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
RUN_DIR = Path("/tmp/actrail-plugin-lifecycle")
SOURCE_CONFIG = ROOT / "tests/plugins/otel-jsonl/operator.conf"
CONFIG = RUN_DIR / "operator.conf"
SOCKET_PATH = RUN_DIR / "actraild.sock"
INSTANCE = "builtin.otel-jsonl"


def run(cmd: list[str], *, timeout: int = 60, check: bool = True) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        cmd,
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=timeout,
    )
    if check and completed.returncode != 0:
        raise RuntimeError(f"command failed {cmd}: exit={completed.returncode}\n{completed.stdout[-4000:]}")
    return completed


def wait_for_socket(path: Path, timeout: float = 10.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.exists():
            return
        time.sleep(0.05)
    raise RuntimeError(f"daemon socket did not appear: {path}")


def write_config() -> None:
    raw = SOURCE_CONFIG.read_text(encoding="utf-8")
    raw = raw.replace("/tmp/actrail-plugin-otel-jsonl", str(RUN_DIR))
    CONFIG.write_text(raw, encoding="utf-8")


def start_daemon(actraild: Path) -> subprocess.Popen[str]:
    daemon = subprocess.Popen(
        [str(actraild), "--config", str(CONFIG), "run"],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    wait_for_socket(SOCKET_PATH)
    return daemon


def stop_daemon(daemon: subprocess.Popen[str]) -> None:
    daemon.send_signal(signal.SIGINT)
    try:
        stdout, _ = daemon.communicate(timeout=10)
    except subprocess.TimeoutExpired:
        daemon.kill()
        stdout, _ = daemon.communicate(timeout=10)
    if daemon.returncode not in (0, -signal.SIGINT):
        raise RuntimeError(f"daemon exited with {daemon.returncode}\n{stdout[-4000:]}")


def assert_list_active(actraild: Path) -> None:
    listing = run([str(actraild), "--config", str(CONFIG), "plugin", "list"])
    if INSTANCE not in listing.stdout:
        raise RuntimeError(f"plugin list did not include {INSTANCE}\n{listing.stdout}")
    if "observation-consumer" not in listing.stdout:
        raise RuntimeError(f"plugin list did not report observation purpose\n{listing.stdout}")
    if "active" not in listing.stdout:
        raise RuntimeError(f"plugin list did not report active state\n{listing.stdout}")


def assert_status_active(actraild: Path) -> None:
    status = run(
        [
            str(actraild),
            "--config",
            str(CONFIG),
            "plugin",
            "status",
            "--instance",
            INSTANCE,
        ]
    )
    for expected in [
        f"instance={INSTANCE}",
        "plugin_id=otel-jsonl",
        "purpose=observation-consumer",
        "runtime=builtin",
        "state=active",
    ]:
        if expected not in status.stdout:
            raise RuntimeError(f"plugin status missing {expected}\n{status.stdout}")


def assert_status_missing(actraild: Path) -> None:
    missing = run(
        [str(actraild), "--config", str(CONFIG), "plugin", "status", "--instance", INSTANCE],
        check=False,
    )
    if missing.returncode == 0:
        raise RuntimeError("plugin status unexpectedly succeeded after unload")
    if "plugin_not_found" not in missing.stdout:
        raise RuntimeError(
            f"plugin status after unload did not report plugin_not_found\n{missing.stdout}"
        )


def main() -> int:
    bin_dir = Path(os.environ.get("ACTRAIL_BIN_DIR", ROOT / "target/release"))
    actraild = bin_dir / "actraild"

    if RUN_DIR.exists():
        shutil.rmtree(RUN_DIR)
    RUN_DIR.mkdir(parents=True)
    write_config()

    daemon: subprocess.Popen[str] | None = start_daemon(actraild)
    try:
        assert_list_active(actraild)
        assert_status_active(actraild)
        unload = run(
            [str(actraild), "--config", str(CONFIG), "plugin", "unload", "--instance", INSTANCE]
        )
        if f"unloaded instance={INSTANCE}" not in unload.stdout:
            raise RuntimeError(f"plugin unload did not report the unloaded instance\n{unload.stdout}")
        assert_status_missing(actraild)

        stop_daemon(daemon)
        daemon = None
        daemon = start_daemon(actraild)
        assert_list_active(actraild)
        assert_status_active(actraild)

        print(f"plugin_lifecycle_instance={INSTANCE}")
        print("plugin_lifecycle_list=ok")
        print("plugin_lifecycle_status=ok")
        print("plugin_lifecycle_restart_restore=ok")
        return 0
    finally:
        if daemon is not None:
            stop_daemon(daemon)


if __name__ == "__main__":
    raise SystemExit(main())
