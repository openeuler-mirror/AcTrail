#!/usr/bin/env python3
from __future__ import annotations

import os
import re
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
RUN_DIR = Path("/tmp/actrail-plugin-wasm-component-payload-read")
SOURCE_CONFIG = ROOT / "tests/payload/http-local/operator.conf"
CONFIG = RUN_DIR / "operator.conf"
SOCKET_PATH = RUN_DIR / "actrail-http-local-test.sock"
MANIFEST = ROOT / "tests/plugins/wasm-component-payload-read/component-payload-read.plugin.toml"
INSTANCE = "wasm.component-payload-read"
WORKLOAD = ROOT / "tests/regression/cases/08-http-llm-projection/workload.py"
MARKER = "ACTRAIL_WASM_COMPONENT_PAYLOAD_READ_PLUGIN_E2E"


def run(
    cmd: list[str],
    *,
    timeout: int = 60,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        cmd,
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=timeout,
    )
    if check and completed.returncode != 0:
        raise RuntimeError(
            f"command failed {cmd}: exit={completed.returncode}\n{completed.stdout[-4000:]}"
        )
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
    raw = raw.replace("/tmp/actrail-http-local-test", str(RUN_DIR / "actrail-http-local-test"))
    raw = raw.replace("[export.runtime]\nenabled = false", "[export.runtime]\nenabled = false")
    raw = raw.replace("[semantic_retention]", "[semantic_retention]\ncontent_owner = \"configured_layers\"")
    raw = raw.replace(
        "[semantic_retention.l4_payload]",
        "[semantic_retention.l4_payload]\nenabled = true\nbody_content = \"retained\"",
    )
    CONFIG.write_text(raw, encoding="utf-8")


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


def workload_argv() -> list[str]:
    return [
        sys.executable,
        str(WORKLOAD),
        "--model",
        "actrail-local-model",
        "--marker",
        MARKER,
        "--path",
        "/chat/completions",
        "--bind-host",
        "127.0.0.1",
        "--bind-port",
        "0",
        "--response-text",
        "ok",
        "--timeout-seconds",
        "5",
        "--request-write-mode",
        "single-syscall",
        "--host-header",
        "local.actrail",
        "--content-type",
        "application/json",
        "--response-read-chunk-bytes",
        "4096",
        "--request-padding-bytes",
        "120000",
    ]


def assert_ungranted_load_fails(actraild: Path) -> None:
    load = run(
        [
            str(actraild),
            "--config",
            str(CONFIG),
            "plugin",
            "load",
            "--manifest",
            str(MANIFEST),
            "--instance",
            f"{INSTANCE}.ungranted",
        ],
        check=False,
    )
    if load.returncode == 0:
        raise RuntimeError("component payload-read plugin load unexpectedly succeeded without grant")
    missing = [value for value in ("plugin_capability", "payload-read") if value not in load.stdout]
    if missing:
        raise RuntimeError(f"component ungranted load error missed {missing}\n{load.stdout}")


def wait_for_observed_record(actraild: Path) -> int:
    deadline = time.time() + 10.0
    status: dict[str, str] = {}
    while time.time() < deadline:
        status = parse_status_fields(
            run(
                [
                    str(actraild),
                    "--config",
                    str(CONFIG),
                    "plugin",
                    "status",
                    "--instance",
                    INSTANCE,
                ]
            ).stdout
        )
        last_error = status.get("last_error", "")
        if last_error not in ("", "none"):
            raise RuntimeError(f"component payload-read plugin reported last_error\n{status}")
        observed = int(status.get("observed_records", "0"))
        if observed > 0:
            break
        time.sleep(0.1)
    observed = int(status.get("observed_records", "0"))
    if observed <= 0:
        raise RuntimeError(f"component payload-read plugin did not observe payload batches\n{status}")
    if status.get("host_grants") != "payload-read:source=syscall":
        raise RuntimeError(f"component payload-read grant not visible in status\n{status}")
    if status_int(status, "payload_read_calls") <= 0:
        raise RuntimeError(f"component payload-read calls were not recorded\n{status}")
    if status_int(status, "payload_read_bytes") < 3:
        raise RuntimeError(f"component payload-read returned bytes were not recorded\n{status}")
    if status_int(status, "payload_read_truncated") <= 0:
        raise RuntimeError(f"component payload-read truncations were not recorded\n{status}")
    if status_int(status, "payload_read_denied") != 0:
        raise RuntimeError(f"component payload-read unexpectedly recorded denied reads\n{status}")
    if status_int(status, "payload_read_latency_total_ns") <= 0:
        raise RuntimeError(f"component payload-read latency was not recorded\n{status}")
    return observed


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
        assert_ungranted_load_fails(actraild)

        run(
            [
                str(actraild),
                "--config",
                str(CONFIG),
                "plugin",
                "load",
                "--manifest",
                str(MANIFEST),
                "--instance",
                INSTANCE,
                "--grant",
                "payload-read:source=syscall",
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
                *workload_argv(),
            ],
            timeout=90,
        )
        if "llm projection workload complete" not in launch.stdout:
            raise RuntimeError(f"workload did not report completion\n{launch.stdout[-4000:]}")
        if not re.search(r"^workload_pid=\d+$", launch.stdout, re.MULTILINE):
            raise RuntimeError(f"workload output missing pid\n{launch.stdout[-4000:]}")

        observed = wait_for_observed_record(actraild)
        run([str(actraild), "--config", str(CONFIG), "plugin", "unload", "--instance", INSTANCE])
        print(f"wasm_component_payload_read_plugin_instance={INSTANCE}")
        print(f"wasm_component_payload_read_plugin_observed_records={observed}")
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
