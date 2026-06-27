#!/usr/bin/env python3
from __future__ import annotations

import os
import shutil
import signal
import subprocess
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
RUN_DIR = Path("/tmp/actrail-plugin-wasm-observation-prefilter")
SOURCE_CONFIG = ROOT / "tests/plugins/otel-jsonl/operator.conf"
CONFIG = RUN_DIR / "operator.conf"
SOCKET_PATH = RUN_DIR / "actraild.sock"
MANIFEST = ROOT / "tests/plugins/wasm-observation-prefilter/prefilter.plugin.toml"
PLUGIN_CONFIG = ROOT / "tests/plugins/wasm-observation-prefilter/prefilter.config.toml"
INSTANCE = "wasm.observation-prefilter"
MARKER = "ACTRAIL_WASM_OBSERVATION_PREFILTER_E2E"


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
    raw = raw.replace("[export.runtime]\nenabled = true", "[export.runtime]\nenabled = false")
    raw = raw.replace("[plugins.startup]\nenabled = true", "[plugins.startup]\nenabled = false")
    CONFIG.write_text(raw, encoding="utf-8")


def parse_status_fields(raw: str) -> dict[str, str]:
    fields: dict[str, str] = {}
    for line in raw.splitlines():
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        fields[key.strip()] = value.strip()
    return fields


def assert_prefiltered_status(actraild: Path) -> dict[str, str]:
    deadline = time.time() + 2.0
    status: dict[str, str] = {}
    while time.time() < deadline:
        status = parse_status_fields(
            run([str(actraild), "--config", str(CONFIG), "plugin", "status", "--instance", INSTANCE]).stdout
        )
        observed = int(status.get("observed_records", "0"))
        dropped = int(status.get("dropped_records", "0"))
        last_error = status.get("last_error", "none")
        if observed > 0 or dropped > 0 or last_error not in ("", "none"):
            raise RuntimeError(f"prefiltered plugin received a semantic-action batch\n{status}")
        time.sleep(0.1)
    return status


def main() -> int:
    bin_dir = Path(os.environ.get("ACTRAIL_BIN_DIR", ROOT / "target/release"))
    actraild = bin_dir / "actraild"
    actrailctl = bin_dir / "actrailctl"

    if RUN_DIR.exists():
        shutil.rmtree(RUN_DIR)
    RUN_DIR.mkdir(parents=True)
    write_config()

    daemon = subprocess.Popen(
        [str(actraild), "--config", str(CONFIG), "run"],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    try:
        wait_for_socket(SOCKET_PATH)

        run(
            [
                str(actraild),
                "--config",
                str(CONFIG),
                "plugin",
                "load",
                "--manifest",
                str(MANIFEST),
                "--plugin-config",
                str(PLUGIN_CONFIG),
                "--instance",
                INSTANCE,
                "--grant",
                "payload-read",
            ]
        )

        launch = run(
            [
                str(actrailctl),
                "--config",
                str(CONFIG),
                "--socket-path",
                str(SOCKET_PATH),
                "launch",
                "--name",
                MARKER,
                "--",
                "bash",
                "-lc",
                f"echo {MARKER}; cat /etc/hostname >/dev/null",
            ],
            timeout=90,
        )
        if MARKER not in launch.stdout:
            raise RuntimeError(f"workload output missing marker\n{launch.stdout[-4000:]}")

        status = assert_prefiltered_status(actraild)
        if status.get("queue_capacity") != "2":
            raise RuntimeError(f"prefiltered plugin status lost queue_capacity=2\n{status}")
        if status.get("host_grants") != "payload-read":
            raise RuntimeError(f"prefiltered plugin status lost payload-read grant\n{status}")
        if status.get("queue_depth") != "0":
            raise RuntimeError(f"prefiltered plugin entered the queue\n{status}")
        run([str(actraild), "--config", str(CONFIG), "plugin", "unload", "--instance", INSTANCE])

        print(f"wasm_observation_prefilter_plugin_instance={INSTANCE}")
        print("wasm_observation_prefilter_observed_records=0")
        print("wasm_observation_prefilter_dropped_records=0")
        return 0
    finally:
        daemon.send_signal(signal.SIGINT)
        try:
            stdout, _ = daemon.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            daemon.kill()
            stdout, _ = daemon.communicate(timeout=10)
        if daemon.returncode not in (0, -signal.SIGINT):
            raise RuntimeError(f"daemon exited with {daemon.returncode}\n{stdout[-4000:]}")


if __name__ == "__main__":
    raise SystemExit(main())
