#!/usr/bin/env python3
from __future__ import annotations

import os
import shutil
import signal
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
RUN_DIR = Path("/tmp/actrail-plugin-wasm-memory-limit")
SOURCE_CONFIG = ROOT / "tests/plugins/otel-jsonl/operator.conf"
CONFIG = RUN_DIR / "operator.conf"
SOCKET_PATH = RUN_DIR / "actraild.sock"
MANIFEST = ROOT / "tests/plugins/wasm-memory-limit/too-large.plugin.toml"
PLUGIN_CONFIG = ROOT / "tests/plugins/wasm-memory-limit/too-large.config.toml"
INSTANCE = "wasm.memory-too-large"


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


def main() -> int:
    bin_dir = Path(os.environ.get("ACTRAIL_BIN_DIR", ROOT / "target/release"))
    actraild = bin_dir / "actraild"

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
            run([str(actraild), "--config", str(CONFIG), "plugin", "unload", "--instance", INSTANCE], check=False)
            raise RuntimeError("WASM plugin loaded despite exceeding wasm_memory_max_bytes")
        if "wasm_runtime" not in load.stdout:
            raise RuntimeError(f"WASM memory-limit load failure did not report wasm_runtime\n{load.stdout}")

        print(f"wasm_memory_limit_plugin_instance={INSTANCE}")
        print("wasm_memory_limit_rejected=ok")
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
