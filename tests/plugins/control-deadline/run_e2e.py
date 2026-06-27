#!/usr/bin/env python3
"""Run the WASM ControlDecider deadline timer isolation E2E."""

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
FIXTURE_DIR = ROOT / "tests/plugins/control-deadline"
MANIFEST = FIXTURE_DIR / "deadline.plugin.toml"
PLUGIN_CONFIG = FIXTURE_DIR / "deadline.config.toml"
INSTANCE = "wasm.graylist-deadline"
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
    parser.add_argument("--agent-timeout-sec", type=float, default=5.0)
    parser.add_argument("--minimum-second-elapsed-ms", type=int, default=1200)
    return parser.parse_args()


def wait_for_agent_output(process: subprocess.Popen[str], timeout_sec: float) -> str:
    deadline = time.monotonic() + timeout_sec
    lines: list[str] = []
    while time.monotonic() < deadline:
        line = helpers.read_line_until(process, process.stdout, deadline)
        if line:
            print(line, end="")
            lines.append(line)
            if line.startswith("second_elapsed_ms="):
                break
        if process.poll() is not None:
            break
    output = "".join(lines)
    try:
        exit_code = process.wait(timeout=max(deadline - time.monotonic(), 0.1))
    except subprocess.TimeoutExpired as error:
        raise RuntimeError("agent did not receive deadline fallback response") from error
    if exit_code != 0:
        stderr = process.stderr.read() if process.stderr else ""
        raise RuntimeError(f"agent failed with exit={exit_code}: {output}{stderr}")
    return output


def parse_elapsed_ms(output: str) -> int:
    for line in output.splitlines():
        if line.startswith("second_elapsed_ms="):
            return int(line.split("=", 1)[1])
    raise RuntimeError(f"missing second_elapsed_ms in agent output\n{output}")


def main() -> int:
    args = parse_args()
    helpers.require_root()
    bin_dir = ROOT / args.bin_dir
    actraild = helpers.require_binary(bin_dir, "actraild")
    actrailctl = helpers.require_binary(bin_dir, "actrailctl")
    agent_script = FIXTURE_DIR / "agent.py"

    with tempfile.TemporaryDirectory(prefix="actrail-control-deadline-e2e-") as raw_tmp:
        tmp = Path(raw_tmp)
        fast = tmp / "targets" / "fast.txt"
        slow = tmp / "targets" / "slow.txt"
        rules = tmp / "rules.conf"
        config = tmp / "operator.conf"
        helpers.write_text(fast, "fast\n")
        helpers.write_text(slow, "slow\n")
        helpers.write_text(
            rules,
            "\n".join(
                [
                    f"fast-file gray sync-plugin {INSTANCE} timeout-ms 500 concurrency 1 fallback deny open {fast}",
                    f"slow-file gray sync-plugin {INSTANCE} timeout-ms 1800 concurrency 1 fallback deny open {slow}",
                    "",
                ]
            ),
        )
        helpers.write_text(config, helpers.operator_config(tmp, rules))

        daemon = subprocess.Popen(
            [str(actraild), "--config", str(config), "run"],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        agent = None
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
                [
                    sys.executable,
                    str(agent_script),
                    "--fast-path",
                    str(fast),
                    "--slow-path",
                    str(slow),
                ],
                text=True,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
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
                    "control-deadline-plugin-e2e",
                ]
            )
            if agent.stdin is None:
                raise RuntimeError("agent stdin is not captured")
            agent.stdin.write("go\n")
            agent.stdin.flush()
            output = wait_for_agent_output(agent, args.agent_timeout_sec)
            if "first=ok" not in output:
                raise RuntimeError(f"first decision did not allow\n{output}")
            if "second=permission_denied" not in output:
                raise RuntimeError(f"second timeout fallback did not deny\n{output}")
            elapsed_ms = parse_elapsed_ms(output)
            if elapsed_ms < args.minimum_second_elapsed_ms:
                raise RuntimeError(
                    "second decision was interrupted before its own deadline: "
                    f"elapsed_ms={elapsed_ms} minimum={args.minimum_second_elapsed_ms}"
                )
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
            last_error = status.get("last_error", "")
            if "timeout after 1800ms" not in last_error:
                raise RuntimeError(f"second timeout did not use its own deadline\n{status}")
        finally:
            if agent is not None:
                helpers.stop_process(agent)
            helpers.stop_process(daemon)

    print(f"control_deadline_plugin_instance={INSTANCE}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"control deadline e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
