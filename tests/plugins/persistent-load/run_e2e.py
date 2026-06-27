#!/usr/bin/env python3
from __future__ import annotations

import json
import os
import shutil
import signal
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
RUN_DIR = Path("/tmp/actrail-plugin-persistent-load")
SOURCE_CONFIG = ROOT / "tests/plugins/otel-jsonl/operator.conf"
CONFIG = RUN_DIR / "operator.conf"
REGISTRY = RUN_DIR / "operator.conf.plugins.toml"
SOCKET_PATH = RUN_DIR / "actraild.sock"
MANIFEST = ROOT / "tests/plugins/dynamic-builtin/otel-jsonl.plugin.toml"
PLUGIN_CONFIG = RUN_DIR / "otel-jsonl.config.toml"
INSTANCE = "persistent.otel-jsonl"
MARKER = "ACTRAIL_PERSISTENT_PLUGIN_E2E"


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
    PLUGIN_CONFIG.write_text(
        "\n".join(
            [
                f'path = "{RUN_DIR / "live-spans.otlp.jsonl"}"',
                "overwrite_enabled = true",
                "queue_capacity = 128",
                "flush_every_spans = 1",
                "",
            ]
        ),
        encoding="utf-8",
    )


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


def status(actraild: Path, *, check: bool = True) -> subprocess.CompletedProcess[str]:
    return run(
        [str(actraild), "--config", str(CONFIG), "plugin", "status", "--instance", INSTANCE],
        check=check,
    )


def parse_status_fields(raw: str) -> dict[str, str]:
    fields: dict[str, str] = {}
    for line in raw.splitlines():
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        fields[key.strip()] = value.strip()
    return fields


def status_int(fields: dict[str, str], key: str) -> int:
    if key not in fields:
        raise RuntimeError(f"plugin status missed {key}\n{fields}")
    try:
        return int(fields[key])
    except ValueError as error:
        raise RuntimeError(f"plugin status {key} is not an integer\n{fields}") from error


def assert_active(actraild: Path) -> None:
    current = status(actraild).stdout
    for expected in [
        f"instance={INSTANCE}",
        "plugin_id=otel-jsonl",
        "purpose=observation-consumer",
        "runtime=builtin",
        "state=active",
    ]:
        if expected not in current:
            raise RuntimeError(f"persistent plugin status missing {expected}\n{current}")


def assert_registry_contains_record() -> None:
    if not REGISTRY.exists():
        raise RuntimeError(f"persistent plugin registry missing: {REGISTRY}")
    registry_raw = REGISTRY.read_text(encoding="utf-8")
    for expected in [INSTANCE, str(MANIFEST), str(PLUGIN_CONFIG)]:
        if expected not in registry_raw:
            raise RuntimeError(f"persistent registry missing {expected}\n{registry_raw}")


def assert_registry_removed() -> None:
    if REGISTRY.exists():
        registry_raw = REGISTRY.read_text(encoding="utf-8")
        raise RuntimeError(f"persistent plugin registry should be removed\n{registry_raw}")


def assert_observed_records(actraild: Path) -> int:
    fields = parse_status_fields(status(actraild).stdout)
    observed = status_int(fields, "observed_records")
    if observed <= 0:
        raise RuntimeError(f"persistent plugin did not observe records\n{fields}")
    if fields.get("last_error", "") not in ("", "none"):
        raise RuntimeError(f"persistent plugin reported last_error\n{fields}")
    return observed


def assert_missing(actraild: Path) -> None:
    missing = status(actraild, check=False)
    if missing.returncode == 0:
        raise RuntimeError("persistent plugin status unexpectedly succeeded")
    if "plugin_not_found" not in missing.stdout:
        raise RuntimeError(f"missing plugin did not report plugin_not_found\n{missing.stdout}")


def read_spans(path: Path) -> list[dict]:
    if not path.exists():
        raise RuntimeError(f"persistent plugin JSONL file missing: {path}")
    spans: list[dict] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.strip():
            spans.append(json.loads(line))
    if not spans:
        raise RuntimeError(f"persistent plugin JSONL file is empty: {path}")
    return spans


def exercise_observation(actrailctl: Path) -> int:
    run(
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
    jsonl_path = RUN_DIR / "live-spans.otlp.jsonl"
    deadline = time.time() + 10.0
    spans: list[dict] = []
    while time.time() < deadline:
        try:
            spans = read_spans(jsonl_path)
            if MARKER in json.dumps(spans):
                break
        except RuntimeError:
            pass
        time.sleep(0.1)
    if MARKER not in json.dumps(spans):
        raise RuntimeError(f"persistent plugin JSONL does not contain marker {MARKER}")
    return len(spans)


def main() -> int:
    bin_dir = Path(os.environ.get("ACTRAIL_BIN_DIR", ROOT / "target/release"))
    actraild = bin_dir / "actraild"
    actrailctl = bin_dir / "actrailctl"

    if RUN_DIR.exists():
        shutil.rmtree(RUN_DIR)
    RUN_DIR.mkdir(parents=True)
    write_config()

    daemon = start_daemon(actraild)
    try:
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
                "--persist",
            ]
        )
        assert_active(actraild)
        assert_registry_contains_record()
    finally:
        stop_daemon(daemon)

    daemon = start_daemon(actraild)
    try:
        assert_active(actraild)
        restored_spans = exercise_observation(actrailctl)
        restored_observed = assert_observed_records(actraild)
        run(
            [
                str(actraild),
                "--config",
                str(CONFIG),
                "plugin",
                "unload",
                "--instance",
                INSTANCE,
            ]
        )
        assert_missing(actraild)
        assert_registry_contains_record()
    finally:
        stop_daemon(daemon)

    daemon = start_daemon(actraild)
    try:
        assert_active(actraild)
        reloaded_spans = exercise_observation(actrailctl)
        reloaded_observed = assert_observed_records(actraild)
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
        assert_missing(actraild)
        assert_registry_removed()
    finally:
        stop_daemon(daemon)

    daemon = start_daemon(actraild)
    try:
        assert_missing(actraild)
    finally:
        stop_daemon(daemon)

    print(f"persistent_plugin_instance={INSTANCE}")
    print(f"persistent_plugin_restored_spans={restored_spans}")
    print(f"persistent_plugin_restored_observed_records={restored_observed}")
    print(f"persistent_plugin_reloaded_spans={reloaded_spans}")
    print(f"persistent_plugin_reloaded_observed_records={reloaded_observed}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
