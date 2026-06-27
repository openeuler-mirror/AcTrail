#!/usr/bin/env python3
from __future__ import annotations

import os
import shutil
import signal
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
RUN_DIR = Path("/tmp/actrail-plugin-wasm-component-observation")
SOURCE_CONFIG = ROOT / "tests/plugins/otel-jsonl/operator.conf"
CONFIG = RUN_DIR / "operator.conf"
SOCKET_PATH = RUN_DIR / "actraild.sock"
MANIFEST = ROOT / "tests/plugins/wasm-component-observation/component.plugin.toml"
CONFIG_MANIFEST = ROOT / "tests/plugins/wasm-component-observation/component-config.plugin.toml"
INSTANCE = "wasm.component-observation"
CONFIG_INSTANCE = "wasm.component-observation-config"
MARKER = "ACTRAIL_WASM_COMPONENT_OBSERVATION_PLUGIN_E2E"


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


def load_component(actraild: Path, manifest: Path, instance: str, plugin_config: Path | None = None) -> None:
    command = [
        str(actraild),
        "--config",
        str(CONFIG),
        "plugin",
        "load",
        "--manifest",
        str(manifest),
        "--instance",
        instance,
    ]
    if plugin_config is not None:
        command.extend(["--plugin-config", str(plugin_config)])
    run(command)


def main() -> int:
    bin_dir = Path(os.environ.get("ACTRAIL_BIN_DIR", ROOT / "target/release"))
    actraild = bin_dir / "actraild"
    actrailctl = bin_dir / "actrailctl"

    if RUN_DIR.exists():
        shutil.rmtree(RUN_DIR)
    RUN_DIR.mkdir(parents=True)
    write_config()
    plugin_config = RUN_DIR / "component-observation.config.toml"
    plugin_config.write_text('mode = "component-config-ok"\n', encoding="utf-8")

    daemon = subprocess.Popen(
        [str(actraild), "--config", str(CONFIG), "run"],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    try:
        wait_for_socket(SOCKET_PATH)

        load_component(actraild, CONFIG_MANIFEST, CONFIG_INSTANCE, plugin_config)
        load_component(actraild, MANIFEST, INSTANCE)

        before = parse_status_fields(
            run([str(actraild), "--config", str(CONFIG), "plugin", "status", "--instance", INSTANCE]).stdout
        )
        if before.get("runtime") != "wasm":
            raise RuntimeError(f"component plugin status did not report runtime=wasm\n{before}")
        if before.get("host_grants") != "none":
            raise RuntimeError(f"component plugin should not have host grants\n{before}")
        config_before = parse_status_fields(
            run(
                [
                    str(actraild),
                    "--config",
                    str(CONFIG),
                    "plugin",
                    "status",
                    "--instance",
                    CONFIG_INSTANCE,
                ]
            ).stdout
        )
        if config_before.get("runtime") != "wasm":
            raise RuntimeError(f"component config plugin status did not report runtime=wasm\n{config_before}")
        if config_before.get("host_grants") != "none":
            raise RuntimeError(f"component config plugin should not have host grants\n{config_before}")

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
        status: dict[str, str] = {}
        while time.time() < deadline:
            status = parse_status_fields(
                run([str(actraild), "--config", str(CONFIG), "plugin", "status", "--instance", INSTANCE]).stdout
            )
            observed = int(status.get("observed_records", "0"))
            if observed > 0:
                break
            time.sleep(0.1)
        observed = int(status.get("observed_records", "0"))
        if observed <= 0:
            raise RuntimeError(f"component plugin did not observe semantic action batches\n{status}")
        if status.get("last_error", "") not in ("", "none"):
            raise RuntimeError(f"component plugin reported last_error\n{status}")
        config_status: dict[str, str] = {}
        deadline = time.time() + 10.0
        while time.time() < deadline:
            config_status = parse_status_fields(
                run(
                    [
                        str(actraild),
                        "--config",
                        str(CONFIG),
                        "plugin",
                        "status",
                        "--instance",
                        CONFIG_INSTANCE,
                    ]
                ).stdout
            )
            config_observed = int(config_status.get("observed_records", "0"))
            if config_observed > 0:
                break
            time.sleep(0.1)
        config_observed = int(config_status.get("observed_records", "0"))
        if config_observed <= 0:
            raise RuntimeError(
                f"component config plugin did not observe semantic action batches\n{config_status}"
            )
        if config_status.get("last_error", "") not in ("", "none"):
            raise RuntimeError(f"component config plugin reported last_error\n{config_status}")

        run([str(actraild), "--config", str(CONFIG), "plugin", "unload", "--instance", CONFIG_INSTANCE])
        run([str(actraild), "--config", str(CONFIG), "plugin", "unload", "--instance", INSTANCE])

        print(f"wasm_component_observation_plugin_instance={INSTANCE}")
        print(f"wasm_component_observation_plugin_observed_records={observed}")
        print(f"wasm_component_observation_config_plugin_instance={CONFIG_INSTANCE}")
        print(f"wasm_component_observation_config_plugin_observed_records={config_observed}")
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
