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
RUN_DIR = Path("/tmp/actrail-plugin-wasm-payload-read")
SOURCE_CONFIG = ROOT / "tests/payload/http-local/operator.conf"
CONFIG = RUN_DIR / "operator.conf"
REGISTRY = RUN_DIR / "operator.conf.plugins.toml"
SOCKET_PATH = RUN_DIR / "actrail-http-local-test.sock"
MANIFEST = ROOT / "tests/plugins/wasm-payload-read/payload-read.plugin.toml"
DENIED_MANIFEST = ROOT / "tests/plugins/wasm-payload-read/payload-read-denied.plugin.toml"
SCOPED_DENIED_MANIFEST = ROOT / "tests/plugins/wasm-payload-read/payload-read-scoped-denied.plugin.toml"
INSTANCE = "wasm.payload-read"
FULL_INSTANCE = "wasm.payload-read-full"
DENIED_INSTANCE = "wasm.payload-read-denied"
SCOPED_DENIED_INSTANCE = "wasm.payload-read-scoped-denied"
WORKLOAD = ROOT / "tests/regression/cases/08-http-llm-projection/workload.py"
MARKER = "ACTRAIL_WASM_PAYLOAD_READ_PLUGIN_E2E"


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
        raise RuntimeError("payload-read plugin load unexpectedly succeeded without grant")
    missing = [value for value in ("plugin_capability", "payload-read") if value not in load.stdout]
    if missing:
        raise RuntimeError(f"ungranted load error missed {missing}\n{load.stdout}")


def assert_registry_contains_scoped_grant() -> None:
    if not REGISTRY.exists():
        raise RuntimeError(f"persistent plugin registry missing: {REGISTRY}")
    registry_raw = REGISTRY.read_text(encoding="utf-8")
    for expected in [INSTANCE, str(MANIFEST), "payload-read:source=syscall"]:
        if expected not in registry_raw:
            raise RuntimeError(f"persistent registry missing {expected}\n{registry_raw}")


def assert_scoped_plugin_restored(actraild: Path) -> None:
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
    if status.get("instance") != INSTANCE:
        raise RuntimeError(f"restored scoped plugin status has wrong instance\n{status}")
    if status.get("host_grants") != "payload-read:source=syscall":
        raise RuntimeError(f"restored scoped plugin status lost host grant\n{status}")
    if status.get("state") != "active":
        raise RuntimeError(f"restored scoped plugin is not active\n{status}")



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


def wait_for_observed_record(actraild: Path, instance: str, expected_host_grants: str) -> int:
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
                    instance,
                ]
            ).stdout
        )
        observed = int(status.get("observed_records", "0"))
        if observed > 0:
            break
        time.sleep(0.1)
    observed = int(status.get("observed_records", "0"))
    if observed <= 0:
        raise RuntimeError(f"payload-read plugin {instance} did not observe batches\n{status}")
    if status.get("last_error", "") not in ("", "none"):
        raise RuntimeError(f"payload-read plugin {instance} reported last_error\n{status}")
    if status.get("host_grants") != expected_host_grants:
        raise RuntimeError(f"payload-read grant not visible in status for {instance}\n{status}")
    calls = status_int(status, "payload_read_calls")
    bytes_returned = status_int(status, "payload_read_bytes")
    denied = status_int(status, "payload_read_denied")
    not_found = status_int(status, "payload_read_not_found")
    invalid = status_int(status, "payload_read_invalid")
    too_large = status_int(status, "payload_read_too_large")
    truncated = status_int(status, "payload_read_truncated")
    latency_total_ns = status_int(status, "payload_read_latency_total_ns")
    latency_max_ns = status_int(status, "payload_read_latency_max_ns")
    if calls <= 0:
        raise RuntimeError(f"payload-read calls were not recorded\n{status}")
    if bytes_returned <= 0:
        raise RuntimeError(f"payload-read returned bytes were not recorded\n{status}")
    if too_large <= 0:
        raise RuntimeError(f"payload-read too-large results were not recorded\n{status}")
    if not_found <= 0:
        raise RuntimeError(f"payload-read not-found results were not recorded\n{status}")
    if invalid <= 0:
        raise RuntimeError(f"payload-read invalid results were not recorded\n{status}")
    if truncated <= 0:
        raise RuntimeError(f"payload-read truncations were not recorded\n{status}")
    if latency_total_ns <= 0 or latency_max_ns <= 0:
        raise RuntimeError(f"payload-read latency was not recorded\n{status}")
    if denied != 0:
        raise RuntimeError(
            f"granted payload-read plugin {instance} unexpectedly recorded denied reads\n{status}"
        )
    return observed


def wait_for_denied_payload_read(actraild: Path) -> int:
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
                    DENIED_INSTANCE,
                ]
            ).stdout
        )
        denied = int(status.get("payload_read_denied", "0"))
        if denied > 0:
            break
        time.sleep(0.1)
    denied = status_int(status, "payload_read_denied")
    if denied <= 0:
        raise RuntimeError(f"payload-read denied results were not recorded\n{status}")
    if status.get("host_grants") != "none":
        raise RuntimeError(f"denied plugin unexpectedly has host grants\n{status}")
    if status_int(status, "payload_read_calls") <= 0:
        raise RuntimeError(f"denied plugin did not record payload-read calls\n{status}")
    if status_int(status, "payload_read_bytes") != 0:
        raise RuntimeError(f"denied plugin unexpectedly recorded returned bytes\n{status}")
    if status_int(status, "payload_read_latency_total_ns") <= 0:
        raise RuntimeError(f"denied plugin did not record payload-read latency\n{status}")
    if status.get("last_error", "") not in ("", "none"):
        raise RuntimeError(f"denied plugin reported last_error\n{status}")
    return denied


def wait_for_scoped_denied_payload_read(actraild: Path) -> int:
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
                    SCOPED_DENIED_INSTANCE,
                ]
            ).stdout
        )
        denied = int(status.get("payload_read_denied", "0"))
        if denied > 0:
            break
        time.sleep(0.1)
    denied = status_int(status, "payload_read_denied")
    if denied <= 0:
        raise RuntimeError(f"scoped payload-read denied results were not recorded\n{status}")
    if status.get("host_grants") != "payload-read:source=stdio":
        raise RuntimeError(f"scoped denied plugin grant not visible in status\n{status}")
    if status_int(status, "payload_read_calls") <= 0:
        raise RuntimeError(f"scoped denied plugin did not record payload-read calls\n{status}")
    if status_int(status, "payload_read_bytes") != 0:
        raise RuntimeError(f"scoped denied plugin unexpectedly recorded returned bytes\n{status}")
    if status.get("last_error", "") not in ("", "none"):
        raise RuntimeError(f"scoped denied plugin reported last_error\n{status}")
    return denied


def main() -> int:
    bin_dir = Path(os.environ.get("ACTRAIL_BIN_DIR", ROOT / "target/release"))
    actraild = bin_dir / "actraild"
    actrailctl = bin_dir / "actrailctl"

    if RUN_DIR.exists():
        shutil.rmtree(RUN_DIR)
    RUN_DIR.mkdir(parents=True)
    write_config()

    daemon: subprocess.Popen[str] | None = start_daemon(actraild)
    try:
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
                "--persist",
            ]
        )
        assert_registry_contains_scoped_grant()
        stop_daemon(daemon)
        daemon = None
        daemon = start_daemon(actraild)
        assert_scoped_plugin_restored(actraild)
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
                FULL_INSTANCE,
                "--grant",
                "payload-read",
            ]
        )
        run(
            [
                str(actraild),
                "--config",
                str(CONFIG),
                "plugin",
                "load",
                "--manifest",
                str(DENIED_MANIFEST),
                "--instance",
                DENIED_INSTANCE,
            ]
        )
        run(
            [
                str(actraild),
                "--config",
                str(CONFIG),
                "plugin",
                "load",
                "--manifest",
                str(SCOPED_DENIED_MANIFEST),
                "--instance",
                SCOPED_DENIED_INSTANCE,
                "--grant",
                "payload-read:source=stdio",
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

        observed = wait_for_observed_record(actraild, INSTANCE, "payload-read:source=syscall")
        full_observed = wait_for_observed_record(actraild, FULL_INSTANCE, "payload-read")
        denied = wait_for_denied_payload_read(actraild)
        scoped_denied = wait_for_scoped_denied_payload_read(actraild)
        run(
            [
                str(actraild),
                "--config",
                str(CONFIG),
                "plugin",
                "unload",
                "--instance",
                INSTANCE,
                "--persist",
            ]
        )
        run(
            [
                str(actraild),
                "--config",
                str(CONFIG),
                "plugin",
                "unload",
                "--instance",
                FULL_INSTANCE,
            ]
        )
        run(
            [
                str(actraild),
                "--config",
                str(CONFIG),
                "plugin",
                "unload",
                "--instance",
                DENIED_INSTANCE,
            ]
        )
        run(
            [
                str(actraild),
                "--config",
                str(CONFIG),
                "plugin",
                "unload",
                "--instance",
                SCOPED_DENIED_INSTANCE,
            ]
        )
        print(f"wasm_payload_read_plugin_instance={INSTANCE}")
        print(f"wasm_payload_read_plugin_observed_records={observed}")
        print(f"wasm_payload_read_full_grant_observed_records={full_observed}")
        print(f"wasm_payload_read_denied_count={denied}")
        print(f"wasm_payload_read_scoped_denied_count={scoped_denied}")
        return 0
    finally:
        if daemon is not None:
            stop_daemon(daemon)


if __name__ == "__main__":
    raise SystemExit(main())
