#!/usr/bin/env python3
"""Run the explicit control plugin instance-capacity E2E."""

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
FIXTURE_DIR = ROOT / "tests/plugins/control-instance-capacity"
TIMEOUT_FIXTURE_DIR = ROOT / "tests/plugins/control-timeout"
MANIFEST = FIXTURE_DIR / "capacity-one.plugin.toml"
PLUGIN_CONFIG = TIMEOUT_FIXTURE_DIR / "timeout.config.toml"
INSTANCE = "wasm.graylist-instance-capacity"
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
    parser.add_argument("--agent-timeout-sec", type=float, default=6.0)
    parser.add_argument("--second-response-timeout-sec", type=float, default=1.5)
    parser.add_argument("--drain-attempts", type=int, default=30)
    parser.add_argument("--drain-sleep-sec", type=float, default=0.2)
    return parser.parse_args()


def wait_for_gray_result(process: subprocess.Popen[str], timeout_sec: float) -> str:
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
    output = "".join(lines)
    if "gray=" not in output:
        raise RuntimeError("agent did not receive graylist response before timeout")
    exit_code = process.wait(timeout=max(deadline - time.monotonic(), 0.1))
    if exit_code != 0:
        stderr = process.stderr.read() if process.stderr else ""
        raise RuntimeError(f"agent failed with exit={exit_code}: {output}{stderr}")
    return output


def start_agent(agent_script: Path, gray: Path) -> subprocess.Popen[str]:
    return subprocess.Popen(
        [sys.executable, str(agent_script), "--gray-path", str(gray)],
        text=True,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def track_agent(actrailctl: Path, config: Path, pid: int, name: str) -> None:
    helpers.run_checked(
        [
            str(actrailctl),
            "--config",
            str(config),
            "track-add",
            "--pid",
            str(pid),
            "--name",
            name,
        ]
    )


def release_agent(agent: subprocess.Popen[str]) -> None:
    if agent.stdin is None:
        raise RuntimeError("agent stdin is not captured")
    agent.stdin.write("go\n")
    agent.stdin.flush()


def wait_for_instance_capacity_audit(
    actrailctl: Path,
    actrailviewer: Path,
    config: Path,
    attempts: int,
    sleep_sec: float,
) -> str:
    expected = [
        "Enforcement",
        "second-gray",
        "decision_source=sync-plugin-fallback",
        f"plugin_instance={INSTANCE}",
        "plugin_concurrency_limit=4",
        "plugin_instance_concurrency_limit=1",
        "fallback_reason=plugin_instance_concurrency_limit",
        "plugin_instance_inflight=1",
    ]
    for _ in range(attempts):
        helpers.run_checked([str(actrailctl), "--config", str(config), "list-traces"])
        combined = []
        for trace_id in ("1", "2"):
            output = helpers.run_checked(
                [
                    str(actrailviewer),
                    "events",
                    "--config",
                    str(config),
                    "--trace-id",
                    trace_id,
                ]
            )
            combined.append(output)
        output = "\n".join(combined)
        if all(value in output for value in expected):
            return output
        time.sleep(sleep_sec)
    raise RuntimeError("actrailviewer did not show expected instance-capacity fallback audit")


def main() -> int:
    args = parse_args()
    helpers.require_root()
    bin_dir = ROOT / args.bin_dir
    actraild = helpers.require_binary(bin_dir, "actraild")
    actrailctl = helpers.require_binary(bin_dir, "actrailctl")
    actrailviewer = helpers.require_binary(bin_dir, "actrailviewer")
    agent_script = ROOT / "tests/plugins/control-fuel/agent.py"

    with tempfile.TemporaryDirectory(prefix="actrail-control-instance-capacity-e2e-") as raw_tmp:
        tmp = Path(raw_tmp)
        first_gray = tmp / "targets" / "first-gray.txt"
        second_gray = tmp / "targets" / "second-gray.txt"
        rules = tmp / "rules.conf"
        config = tmp / "operator.conf"
        helpers.write_text(first_gray, "first\n")
        helpers.write_text(second_gray, "second\n")
        helpers.write_text(
            rules,
            "\n".join(
                [
                    f"first-gray gray sync-plugin {INSTANCE} timeout-ms 3000 concurrency 4 fallback deny open {first_gray}",
                    f"second-gray gray sync-plugin {INSTANCE} timeout-ms 3000 concurrency 4 fallback deny open {second_gray}",
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
        first = None
        second = None
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

            first = start_agent(agent_script, first_gray)
            first_pid = helpers.read_agent_pid(first, args.agent_timeout_sec)
            track_agent(actrailctl, config, first_pid, "control-instance-capacity-first")
            release_agent(first)
            time.sleep(0.25)

            second = start_agent(agent_script, second_gray)
            second_pid = helpers.read_agent_pid(second, args.agent_timeout_sec)
            track_agent(actrailctl, config, second_pid, "control-instance-capacity-second")
            release_agent(second)

            second_output = wait_for_gray_result(second, args.second_response_timeout_sec)
            if "gray=permission_denied" not in second_output:
                raise RuntimeError(
                    f"second agent was not denied by instance-capacity fallback\n{second_output}"
                )

            first_output = wait_for_gray_result(first, args.agent_timeout_sec)
            if "gray=permission_denied" not in first_output:
                raise RuntimeError(f"first agent was not denied by plugin timeout fallback\n{first_output}")

            viewer_output = wait_for_instance_capacity_audit(
                actrailctl,
                actrailviewer,
                config,
                args.drain_attempts,
                args.drain_sleep_sec,
            )
            if "fallback_reason=plugin_error" not in viewer_output:
                raise RuntimeError("first slow-path timeout audit was not recorded")
        finally:
            if first is not None:
                helpers.stop_process(first)
            if second is not None:
                helpers.stop_process(second)
            helpers.stop_process(daemon)

    print(f"control_instance_capacity_plugin_instance={INSTANCE}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"control instance-capacity e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
