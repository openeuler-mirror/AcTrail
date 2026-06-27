#!/usr/bin/env python3
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
RUN_DIR = Path("/tmp/actrail-plugin-dynamic-builtin")
SOURCE_CONFIG = ROOT / "tests/plugins/otel-jsonl/operator.conf"
CONFIG = RUN_DIR / "operator.conf"
SOCKET_PATH = RUN_DIR / "actraild.sock"
MANIFEST = ROOT / "tests/plugins/dynamic-builtin/otel-jsonl.plugin.toml"
PLUGIN_CONFIG = ROOT / "tests/plugins/dynamic-builtin/otel-jsonl.config.toml"
INSTANCE = "dynamic.otel-jsonl"
MARKER = "ACTRAIL_DYNAMIC_BUILTIN_PLUGIN_E2E"


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


def read_spans(path: Path) -> list[dict]:
    if not path.exists():
        raise RuntimeError(f"dynamic plugin JSONL file missing: {path}")
    spans: list[dict] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.strip():
            spans.append(json.loads(line))
    if not spans:
        raise RuntimeError(f"dynamic plugin JSONL file is empty: {path}")
    return spans


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


def assert_no_startup_builtin(actraild: Path) -> None:
    listing = run([str(actraild), "--config", str(CONFIG), "plugin", "list"]).stdout
    for unexpected in ["builtin.otel-jsonl", INSTANCE]:
        if unexpected in listing:
            raise RuntimeError(
                f"dynamic builtin E2E must start without {unexpected}\n{listing}"
            )


def assert_dynamic_plugin_observed(actraild: Path) -> int:
    status = parse_status_fields(
        run(
            [str(actraild), "--config", str(CONFIG), "plugin", "status", "--instance", INSTANCE]
        ).stdout
    )
    observed = status_int(status, "observed_records")
    if observed <= 0:
        raise RuntimeError(f"dynamic plugin did not observe records\n{status}")
    if status.get("last_error", "") not in ("", "none"):
        raise RuntimeError(f"dynamic plugin reported last_error\n{status}")
    return observed


def assert_duplicate_instance_load_fails(actraild: Path) -> None:
    load = run(
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
        ],
        check=False,
    )
    if load.returncode == 0:
        raise RuntimeError("duplicate dynamic plugin load unexpectedly succeeded")
    missing = [
        value
        for value in ("plugin_runtime", "already exists", INSTANCE)
        if value not in load.stdout
    ]
    if missing:
        raise RuntimeError(f"duplicate load error missed {missing}\n{load.stdout}")


def main() -> int:
    bin_dir = Path(os.environ.get("ACTRAIL_BIN_DIR", ROOT / "target/release"))
    actraild = bin_dir / "actraild"
    actrailctl = bin_dir / "actrailctl"
    jsonl_path = RUN_DIR / "live-spans.otlp.jsonl"
    db_path = RUN_DIR / "actrail.sqlite"

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
        assert_no_startup_builtin(actraild)

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
        assert_duplicate_instance_load_fails(actraild)

        status = run(
            [str(actraild), "--config", str(CONFIG), "plugin", "status", "--instance", INSTANCE]
        )
        for expected in [
            f"instance={INSTANCE}",
            "plugin_id=otel-jsonl",
            "purpose=observation-consumer",
            "runtime=builtin",
            "state=active",
        ]:
            if expected not in status.stdout:
                raise RuntimeError(f"dynamic plugin status missing {expected}\n{status.stdout}")

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
            raise RuntimeError(f"dynamic plugin JSONL does not contain marker {MARKER}")

        with sqlite3.connect(db_path) as db:
            semantic_actions = db.execute("SELECT COUNT(*) FROM semantic_actions").fetchone()[0]
        if semantic_actions <= 0:
            raise RuntimeError("dynamic plugin workload produced no semantic actions")
        observed = assert_dynamic_plugin_observed(actraild)

        run([str(actraild), "--config", str(CONFIG), "plugin", "unload", "--instance", INSTANCE])
        missing = run(
            [str(actraild), "--config", str(CONFIG), "plugin", "status", "--instance", INSTANCE],
            check=False,
        )
        if missing.returncode == 0:
            raise RuntimeError("dynamic plugin status unexpectedly succeeded after unload")
        if "plugin_not_found" not in missing.stdout:
            raise RuntimeError(f"status after unload did not report plugin_not_found\n{missing.stdout}")

        print(f"dynamic_builtin_plugin_instance={INSTANCE}")
        print(f"dynamic_builtin_plugin_spans={len(spans)}")
        print(f"dynamic_builtin_plugin_observed_records={observed}")
        print(f"dynamic_builtin_plugin_semantic_actions={semantic_actions}")
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
