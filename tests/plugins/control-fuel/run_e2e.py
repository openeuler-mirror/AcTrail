#!/usr/bin/env python3
"""Run the WASM ControlDecider fuel-exhaustion E2E."""

from __future__ import annotations

import argparse
import importlib.util
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
FIXTURE_DIR = ROOT / "tests/plugins/control-fuel"
MANIFEST = FIXTURE_DIR / "spin.plugin.toml"
PLUGIN_CONFIG = FIXTURE_DIR / "spin.config.toml"
INSTANCE = "wasm.graylist-spin"
HELPERS = ROOT / "tests/plugins/control-graylist/run_e2e.py"


def load_helpers():
    spec = importlib.util.spec_from_file_location("control_graylist_helpers", HELPERS)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load helper module {HELPERS}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


helpers = load_helpers()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--daemon-ready-timeout-sec", type=float, default=10.0)
    parser.add_argument("--agent-timeout-sec", type=float, default=3.0)
    return parser.parse_args()


def wait_for_agent_result(process: subprocess.Popen[str], timeout_sec: float) -> str:
    deadline = time.monotonic() + timeout_sec
    lines: list[str] = []
    while time.monotonic() < deadline:
        line = helpers.read_line_until(process, process.stdout, deadline)
        if line:
            print(line, end="")
            lines.append(line)
            if line.startswith("gray="):
                break
        if process.poll() is not None:
            break
    try:
        exit_code = process.wait(timeout=max(deadline - time.monotonic(), 0.1))
    except subprocess.TimeoutExpired as error:
        raise RuntimeError("agent did not receive graylist fallback response") from error
    output = "".join(lines)
    if exit_code != 0:
        stderr = process.stderr.read() if process.stderr else ""
        raise RuntimeError(f"agent failed with exit={exit_code}: {output}{stderr}")
    return output


def main() -> int:
    args = parse_args()
    helpers.require_root()
    bin_dir = ROOT / args.bin_dir
    actraild = helpers.require_binary(bin_dir, "actraild")
    actrailctl = helpers.require_binary(bin_dir, "actrailctl")
    agent_script = FIXTURE_DIR / "agent.py"

    with tempfile.TemporaryDirectory(prefix="actrail-control-fuel-e2e-") as raw_tmp:
        tmp = Path(raw_tmp)
        gray = tmp / "targets" / "gray.txt"
        rules = tmp / "rules.conf"
        config = tmp / "operator.conf"
        helpers.write_text(gray, "gray\n")
        helpers.write_text(
            rules,
            f"gray-file gray sync-plugin {INSTANCE} timeout-ms 5000 concurrency 1 fallback deny open {gray}\n",
        )
        helpers.write_text(config, helpers.operator_config(tmp, rules))

        daemon = subprocess.Popen(
            [str(actraild), "--config", str(config), "run"],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        try:
            helpers.wait_for_daemon(daemon, args.daemon_ready_timeout_sec)
            helpers.run_checked(
                [
                    str(actraild),
                    "--config",
                    str(config),
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
            agent = subprocess.Popen(
                [sys.executable, str(agent_script), "--gray-path", str(gray)],
                text=True,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            try:
                pid = helpers.read_agent_pid(agent, args.agent_timeout_sec)
                helpers.run_checked(
                    [
                        str(actrailctl),
                        "--config",
                        str(config),
                        "track-add",
                        "--pid",
                        str(pid),
                        "--name",
                        "control-fuel-plugin-e2e",
                    ]
                )
                if agent.stdin is None:
                    raise RuntimeError("agent stdin is not captured")
                agent.stdin.write("go\n")
                agent.stdin.flush()
                output = wait_for_agent_result(agent, args.agent_timeout_sec)
                if "gray=permission_denied" not in output:
                    raise RuntimeError(f"gray fallback did not deny access\n{output}")
                status = helpers.parse_status_fields(
                    helpers.run_checked(
                        [
                            str(actraild),
                            "--config",
                            str(config),
                            "plugin",
                            "status",
                            "--instance",
                            INSTANCE,
                        ]
                    )
                )
                dropped = int(status.get("dropped_records", "0"))
                if dropped <= 0:
                    raise RuntimeError(f"fuel exhaustion was not recorded as dropped\n{status}")
                last_error = status.get("last_error", "")
                if "fuel" not in last_error:
                    raise RuntimeError(f"fuel exhaustion error was not reported\n{status}")
            finally:
                helpers.stop_process(agent)
        finally:
            helpers.stop_process(daemon)

    print(f"control_fuel_plugin_instance={INSTANCE}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"control fuel e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
