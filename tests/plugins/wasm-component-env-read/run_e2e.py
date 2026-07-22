#!/usr/bin/env python3
from __future__ import annotations

import os
import shutil
import signal
import subprocess
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
RUN_DIR = Path("/tmp/actrail-plugin-wasm-component-env-read")
SOURCE_CONFIG = ROOT / "tests/plugins/otel-jsonl/operator.conf"
CONFIG = RUN_DIR / "operator.conf"
SOCKET_PATH = RUN_DIR / "actraild.sock"
MANIFEST = ROOT / "tests/plugins/wasm-component-env-read/component-env-read.plugin.toml"
UNSUPPORTED_GRANT_MANIFEST = (
    ROOT / "tests/plugins/wasm-component-env-read/component-context-query-unsupported.plugin.toml"
)
NO_CAPABILITY_MANIFEST = ROOT / "tests/plugins/wasm-component-observation/component.plugin.toml"
INSTANCE = "wasm.component-env-read"
ENV_NAME = "ACTRAIL_COMPONENT_ENV_SECRET"
ENV_VALUE = "component-secret"
MARKER = "ACTRAIL_WASM_COMPONENT_ENV_READ_PLUGIN_E2E"


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
        raise RuntimeError("component env-read plugin load unexpectedly succeeded without grant")
    missing = [value for value in ("plugin_capability", "env-read") if value not in load.stdout]
    if missing:
        raise RuntimeError(f"ungranted component load error missed {missing}\n{load.stdout}")


def assert_unrequested_grant_fails(actraild: Path) -> None:
    load = run(
        [
            str(actraild),
            "--config",
            str(CONFIG),
            "plugin",
            "load",
            "--manifest",
            str(NO_CAPABILITY_MANIFEST),
            "--instance",
            f"{INSTANCE}.unrequested",
            "--grant",
            f"env-read:{ENV_NAME}",
        ],
        check=False,
    )
    if load.returncode == 0:
        raise RuntimeError("component plugin load unexpectedly accepted an unrequested env-read grant")
    missing = [value for value in ("plugin_capability", "did not request", "env-read") if value not in load.stdout]
    if missing:
        raise RuntimeError(f"unrequested component grant error missed {missing}\n{load.stdout}")


def assert_unsupported_component_grant_fails(actraild: Path) -> None:
    load = run(
        [
            str(actraild),
            "--config",
            str(CONFIG),
            "plugin",
            "load",
            "--manifest",
            str(UNSUPPORTED_GRANT_MANIFEST),
            "--instance",
            f"{INSTANCE}.unsupported",
            "--grant",
            "context-query",
        ],
        check=False,
    )
    if load.returncode == 0:
        raise RuntimeError("component plugin load unexpectedly accepted context-query grant")
    missing = [
        value
        for value in (
            "wasm_runtime",
            "requested WIT component host grants",
            "not implemented",
        )
        if value not in load.stdout
    ]
    if missing:
        raise RuntimeError(f"unsupported component grant error missed {missing}\n{load.stdout}")


def assert_grant_is_auditable(actraild: Path) -> None:
    status_raw = run(
        [str(actraild), "--config", str(CONFIG), "plugin", "status", "--instance", INSTANCE]
    ).stdout
    status = parse_status_fields(status_raw)
    expected = f"env-read:{ENV_NAME}"
    if status.get("host_grants") != expected:
        raise RuntimeError(f"component plugin status did not expose host grant {expected}\n{status_raw}")
    list_raw = run([str(actraild), "--config", str(CONFIG), "plugin", "list"]).stdout
    if expected not in list_raw:
        raise RuntimeError(f"component plugin list did not expose host grant {expected}\n{list_raw}")


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
            raise RuntimeError(f"component env-read plugin reported last_error\n{status}")
        observed = int(status.get("observed_records", "0"))
        if observed > 0:
            return observed
        time.sleep(0.1)
    raise RuntimeError(f"component env-read plugin did not observe batches\n{status}")


def main() -> int:
    bin_dir = Path(os.environ.get("ACTRAIL_BIN_DIR", ROOT / "target/release"))
    actraild = bin_dir / "actraild"
    actrailctl = bin_dir / "actrailctl"

    if RUN_DIR.exists():
        shutil.rmtree(RUN_DIR)
    RUN_DIR.mkdir(parents=True)
    write_config()

    daemon_env = os.environ.copy()
    daemon_env[ENV_NAME] = ENV_VALUE
    daemon = subprocess.Popen(
        [str(actraild), "--config", str(CONFIG), "run"],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        env=daemon_env,
    )
    try:
        wait_for_socket(SOCKET_PATH)
        assert_ungranted_load_fails(actraild)
        assert_unrequested_grant_fails(actraild)
        assert_unsupported_component_grant_fails(actraild)

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
                f"env-read:{ENV_NAME}",
            ]
        )
        assert_grant_is_auditable(actraild)

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

        observed = wait_for_observed_record(actraild)
        run([str(actraild), "--config", str(CONFIG), "plugin", "unload", "--instance", INSTANCE])
        print(f"wasm_component_env_read_plugin_instance={INSTANCE}")
        print(f"wasm_component_env_read_plugin_observed_records={observed}")
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
