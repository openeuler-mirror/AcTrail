#!/usr/bin/env python3
from __future__ import annotations

import os
import shutil
import signal
import sqlite3
import subprocess
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
RUN_DIR = Path("/tmp/actrail-plugin-wasm-observation-queue")
SOURCE_CONFIG = ROOT / "tests/plugins/otel-jsonl/operator.conf"
CONFIG = RUN_DIR / "operator.conf"
SOCKET_PATH = RUN_DIR / "actraild.sock"
DB_PATH = RUN_DIR / "actrail.sqlite"
MANIFEST = ROOT / "tests/plugins/wasm-observation-queue/queue.plugin.toml"
SLOW_MANIFEST = ROOT / "tests/plugins/wasm-observation-queue/slow.plugin.toml"
ERROR_MANIFEST = ROOT / "tests/plugins/wasm-observation-queue/error.plugin.toml"
PLUGIN_CONFIG = ROOT / "tests/plugins/wasm-observation-queue/queue.config.toml"
INSTANCE = "wasm.observation-queue"
SLOW_INSTANCE = "wasm.observation-slow-queue"
ERROR_INSTANCE = "wasm.observation-error-queue"
MARKER = "ACTRAIL_WASM_OBSERVATION_QUEUE_E2E"
SLOW_MARKER = "ACTRAIL_WASM_OBSERVATION_QUEUE_FULL_E2E"
ERROR_MARKER = "ACTRAIL_WASM_OBSERVATION_FINAL_DRAIN_E2E"


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


def decode_metadata(raw: str) -> dict[str, str]:
    metadata: dict[str, str] = {}
    for line in raw.splitlines():
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        metadata[unescape_metadata(key)] = unescape_metadata(value)
    return metadata


def unescape_metadata(raw: str) -> str:
    output: list[str] = []
    index = 0
    while index < len(raw):
        ch = raw[index]
        if ch != "\\" or index + 1 >= len(raw):
            output.append(ch)
            index += 1
            continue
        escaped = raw[index + 1]
        if escaped == "n":
            output.append("\n")
        elif escaped == "e":
            output.append("=")
        elif escaped == "\\":
            output.append("\\")
        else:
            output.append("\\")
            output.append(escaped)
        index += 2
    return "".join(output)


def has_queue_full_diagnostic() -> bool:
    with sqlite3.connect(DB_PATH) as db:
        rows = db.execute(
            "SELECT kind, metadata FROM diagnostics WHERE kind = 'runtime_dropped'"
        ).fetchall()
    for kind, raw_metadata in rows:
        metadata = decode_metadata(raw_metadata)
        if (
            kind == "runtime_dropped"
            and metadata.get("exporter") == SLOW_INSTANCE
            and metadata.get("reason") == "observation_queue_full"
            and metadata.get("queue_capacity") == "1"
            and int(metadata.get("dropped_records", "0")) > 0
        ):
            return True
    return False


def has_final_drain_error_diagnostic() -> bool:
    with sqlite3.connect(DB_PATH) as db:
        rows = db.execute(
            "SELECT kind, metadata FROM diagnostics WHERE kind = 'runtime_dropped'"
        ).fetchall()
    for kind, raw_metadata in rows:
        metadata = decode_metadata(raw_metadata)
        if (
            kind == "runtime_dropped"
            and metadata.get("exporter") == ERROR_INSTANCE
            and "wasm observation consume returned -7" in metadata.get("reason", "")
            and metadata.get("queue_capacity") == "2"
            and int(metadata.get("dropped_records", "0")) > 0
        ):
            return True
    return False


def wait_for_dropped_status(actraild: Path, instance: str) -> dict[str, str]:
    deadline = time.time() + 10.0
    status: dict[str, str] = {}
    while time.time() < deadline:
        status = parse_status_fields(
            run([str(actraild), "--config", str(CONFIG), "plugin", "status", "--instance", instance]).stdout
        )
        if int(status.get("dropped_records", "0")) > 0:
            return status
        time.sleep(0.1)
    raise RuntimeError(f"queued observation plugin did not report dropped records\n{status}")


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
            ]
        )

        status = parse_status_fields(
            run([str(actraild), "--config", str(CONFIG), "plugin", "status", "--instance", INSTANCE]).stdout
        )
        if status.get("queue_capacity") != "3":
            raise RuntimeError(f"plugin status did not expose queue_capacity=3\n{status}")
        if status.get("queue_depth") != "0":
            raise RuntimeError(f"plugin status did not expose queue_depth=0 after load\n{status}")

        plugin_list = run([str(actraild), "--config", str(CONFIG), "plugin", "list"]).stdout
        if "QUEUE" not in plugin_list or INSTANCE not in plugin_list or "0/3" not in plugin_list:
            raise RuntimeError(f"plugin list did not expose queue depth/capacity\n{plugin_list}")

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

        deadline = time.time() + 10.0
        observed = 0
        while time.time() < deadline:
            status = parse_status_fields(
                run([str(actraild), "--config", str(CONFIG), "plugin", "status", "--instance", INSTANCE]).stdout
            )
            observed = int(status.get("observed_records", "0"))
            if observed > 0:
                break
            time.sleep(0.1)
        if observed <= 0:
            raise RuntimeError(f"queued observation plugin did not consume records\n{status}")
        if status.get("queue_capacity") != "3":
            raise RuntimeError(f"plugin status lost queue_capacity after workload\n{status}")
        if "queue_depth" not in status:
            raise RuntimeError(f"plugin status lost queue_depth after workload\n{status}")

        run([str(actraild), "--config", str(CONFIG), "plugin", "unload", "--instance", INSTANCE])

        run(
            [
                str(actraild),
                "--config",
                str(CONFIG),
                "plugin",
                "load",
                "--manifest",
                str(SLOW_MANIFEST),
                "--plugin-config",
                str(PLUGIN_CONFIG),
                "--instance",
                SLOW_INSTANCE,
            ]
        )
        for index in range(8):
            slow_launch = run(
                [
                    str(actrailctl),
                    "--config",
                    str(CONFIG),
                    "--socket-path",
                    str(SOCKET_PATH),
                    "launch",
                    "--name",
                    f"{SLOW_MARKER}-{index}",
                    "--",
                    "bash",
                    "-lc",
                    f"echo {SLOW_MARKER}-{index}; for item in $(seq 1 5); do cat /etc/hostname >/dev/null; done",
                ],
                timeout=90,
            )
            if SLOW_MARKER not in slow_launch.stdout:
                raise RuntimeError(f"slow queue workload output missing marker\n{slow_launch.stdout[-4000:]}")
        slow_status = wait_for_dropped_status(actraild, SLOW_INSTANCE)
        if slow_status.get("queue_capacity") != "1":
            raise RuntimeError(f"slow plugin status lost queue_capacity=1\n{slow_status}")
        if not has_queue_full_diagnostic():
            raise RuntimeError("queue-full drop diagnostic was not persisted with expected metadata")

        run([str(actraild), "--config", str(CONFIG), "plugin", "unload", "--instance", SLOW_INSTANCE])

        run(
            [
                str(actraild),
                "--config",
                str(CONFIG),
                "plugin",
                "load",
                "--manifest",
                str(ERROR_MANIFEST),
                "--plugin-config",
                str(PLUGIN_CONFIG),
                "--instance",
                ERROR_INSTANCE,
            ]
        )
        error_launch = run(
            [
                str(actrailctl),
                "--config",
                str(CONFIG),
                "--socket-path",
                str(SOCKET_PATH),
                "launch",
                "--name",
                ERROR_MARKER,
                "--",
                "bash",
                "-lc",
                f"echo {ERROR_MARKER}; cat /etc/hostname >/dev/null",
            ],
            timeout=90,
        )
        if ERROR_MARKER not in error_launch.stdout:
            raise RuntimeError(f"error queue workload output missing marker\n{error_launch.stdout[-4000:]}")
        error_status = wait_for_dropped_status(actraild, ERROR_INSTANCE)
        if error_status.get("queue_capacity") != "2":
            raise RuntimeError(f"error plugin status lost queue_capacity=2\n{error_status}")
        if has_final_drain_error_diagnostic():
            raise RuntimeError("final queued worker drop diagnostic was persisted before unload")
        run([str(actraild), "--config", str(CONFIG), "plugin", "unload", "--instance", ERROR_INSTANCE])
        if not has_final_drain_error_diagnostic():
            raise RuntimeError("final queued worker drop diagnostic was not persisted on unload")

        print(f"wasm_observation_queue_plugin_instance={INSTANCE}")
        print(f"wasm_observation_queue_plugin_observed_records={observed}")
        print(f"wasm_observation_queue_capacity={status['queue_capacity']}")
        print(f"wasm_observation_queue_full_dropped_records={slow_status['dropped_records']}")
        print(f"wasm_observation_queue_final_drain_dropped_records={error_status['dropped_records']}")
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
