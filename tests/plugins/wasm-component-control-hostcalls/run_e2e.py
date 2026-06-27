#!/usr/bin/env python3
"""Run the WIT component ControlDecider read-only hostcall E2E."""

from __future__ import annotations

import argparse
import importlib.util
import os
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
FIXTURE_DIR = ROOT / "tests/plugins/wasm-component-control-hostcalls"
GRAYLIST_DIR = ROOT / "tests/plugins/control-graylist"
MANIFEST = FIXTURE_DIR / "component-hostcalls.plugin.toml"
INSTANCE = "wasm.component-control-hostcalls"


def load_graylist_helpers():
    helper_path = GRAYLIST_DIR / "run_e2e.py"
    spec = importlib.util.spec_from_file_location("control_graylist_e2e", helper_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load helper module {helper_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


GRAYLIST = load_graylist_helpers()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--daemon-ready-timeout-sec", type=float, default=10.0)
    parser.add_argument("--agent-timeout-sec", type=float, default=10.0)
    parser.add_argument("--drain-attempts", type=int, default=20)
    parser.add_argument("--drain-sleep-sec", type=float, default=0.2)
    return parser.parse_args()


def wait_for_events(
    actrailctl: Path,
    actrailviewer: Path,
    config_path: Path,
    attempts: int,
    sleep_sec: float,
) -> str:
    expected = [
        "allow-file",
        "gray-file",
        "deny-file",
        "decision_source=rule",
        "decision_source=sync-plugin",
        "plugin-gray-allow",
        f"plugin_instance={INSTANCE}",
        "plugin_timeout_ms=5000",
        "plugin_concurrency_limit=1",
    ]
    for _ in range(attempts):
        GRAYLIST.run_checked([str(actrailctl), "--config", str(config_path), "list-traces"])
        output = GRAYLIST.run_checked(
            [str(actrailviewer), "events", "--config", str(config_path), "--trace-id", "1"]
        )
        if all(value in output for value in expected):
            return output
        time.sleep(sleep_sec)
    raise RuntimeError("actrailviewer did not show expected component hostcall events")


def stop_process(process: subprocess.Popen[str]) -> None:
    if process.poll() is not None:
        return
    process.send_signal(signal.SIGTERM)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def main() -> int:
    args = parse_args()
    GRAYLIST.require_root()
    bin_dir = ROOT / args.bin_dir
    actraild = GRAYLIST.require_binary(bin_dir, "actraild")
    actrailctl = GRAYLIST.require_binary(bin_dir, "actrailctl")
    actrailviewer = GRAYLIST.require_binary(bin_dir, "actrailviewer")
    agent_script = GRAYLIST_DIR / "agent.py"

    with tempfile.TemporaryDirectory(prefix="actrail-component-control-hostcalls-e2e-") as raw_tmp:
        tmp = Path(raw_tmp)
        allowed = tmp / "targets" / "allowed.txt"
        gray = tmp / "targets" / "gray.txt"
        denied = tmp / "targets" / "denied.txt"
        rules = tmp / "rules.conf"
        config = tmp / "operator.conf"
        GRAYLIST.write_text(allowed, "allowed\n")
        GRAYLIST.write_text(gray, "gray\n")
        GRAYLIST.write_text(denied, "denied\n")
        GRAYLIST.write_text(
            rules,
            "\n".join(
                [
                    f"allow-file allow open {allowed}",
                    f"gray-file gray sync-plugin {INSTANCE} timeout-ms 5000 concurrency 1 fallback deny open {gray}",
                    f"deny-file deny open {denied}",
                    "",
                ]
            ),
        )
        GRAYLIST.write_text(config, GRAYLIST.operator_config(tmp, rules))

        daemon = subprocess.Popen(
            [str(actraild), "--config", str(config), "run"],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        try:
            GRAYLIST.wait_for_daemon(daemon, args.daemon_ready_timeout_sec)
            GRAYLIST.run_checked(
                [
                    str(actraild),
                    "--config",
                    str(config),
                    "plugin",
                    "load",
                    "--manifest",
                    str(MANIFEST),
                    "--grant",
                    "context-query",
                    "--grant",
                    "file-policy-read",
                    "--grant",
                    "file-policy-write",
                    "--instance",
                    INSTANCE,
                ]
            )
            agent = subprocess.Popen(
                [
                    sys.executable,
                    str(agent_script),
                    "--allowed-path",
                    str(allowed),
                    "--gray-path",
                    str(gray),
                    "--denied-path",
                    str(denied),
                    "--gray-repeat",
                    "2",
                ],
                text=True,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            try:
                pid = GRAYLIST.read_agent_pid(agent, args.agent_timeout_sec)
                GRAYLIST.run_checked(
                    [
                        str(actrailctl),
                        "--config",
                        str(config),
                        "track-add",
                        "--pid",
                        str(pid),
                        "--name",
                        "component-control-hostcalls-e2e",
                    ]
                )
                if agent.stdin is None:
                    raise RuntimeError("agent stdin is not captured")
                agent.stdin.write("go\n")
                agent.stdin.flush()
                agent_output = GRAYLIST.wait_for_agent_output(agent, args.agent_timeout_sec)
                if "allowed=ok" not in agent_output:
                    raise RuntimeError("agent did not confirm allow fast path")
                if "gray=ok" not in agent_output:
                    raise RuntimeError("agent did not confirm gray component hostcall allow")
                if "gray2=ok" not in agent_output:
                    raise RuntimeError("agent did not confirm file-policy-write local allow")
                if "denied=permission_denied" not in agent_output:
                    raise RuntimeError("agent did not confirm deny fast path")
                viewer_output = wait_for_events(
                    actrailctl,
                    actrailviewer,
                    config,
                    args.drain_attempts,
                    args.drain_sleep_sec,
                )
                if "component hostcalls allowed gray file" not in viewer_output:
                    raise RuntimeError("viewer output missed component hostcall reason")
                status = GRAYLIST.parse_status_fields(
                    GRAYLIST.run_checked(
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
                if status.get("host_grants") != "context-query,file-policy-read,file-policy-write":
                    raise RuntimeError(f"component host grants missing from status\n{status}")
                if int(status.get("observed_records", "0")) != 1:
                    raise RuntimeError(f"component plugin should only see gray decision\n{status}")
                if status.get("last_error", "") not in ("", "none"):
                    raise RuntimeError(f"component plugin reported last_error\n{status}")
                GRAYLIST.run_checked(
                    [
                        str(actraild),
                        "--config",
                        str(config),
                        "plugin",
                        "unload",
                        "--instance",
                        INSTANCE,
                    ]
                )
            finally:
                stop_process(agent)
        finally:
            stop_process(daemon)

    print(f"wasm_component_control_hostcalls_plugin_instance={INSTANCE}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"component control hostcalls e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
