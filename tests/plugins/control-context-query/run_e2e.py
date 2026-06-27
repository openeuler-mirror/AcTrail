#!/usr/bin/env python3
"""Run the WASM context-query hostcall E2E."""

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
FIXTURE_DIR = ROOT / "tests/plugins/control-context-query"
GRAYLIST_DIR = ROOT / "tests/plugins/control-graylist"
MANIFEST = FIXTURE_DIR / "context-query.plugin.toml"
INSTANCE = "wasm.context-query"


def load_graylist_helpers():
    helper_path = GRAYLIST_DIR / "run_e2e.py"
    spec = importlib.util.spec_from_file_location("control_graylist_e2e", helper_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot import helper module from {helper_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


cg = load_graylist_helpers()


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
        f"plugin_instance={INSTANCE}",
    ]
    for _ in range(attempts):
        cg.run_checked([str(actrailctl), "--config", str(config_path), "list-traces"])
        output = cg.run_checked(
            [str(actrailviewer), "events", "--config", str(config_path), "--trace-id", "1"]
        )
        if all(value in output for value in expected):
            return output
        time.sleep(sleep_sec)
    raise RuntimeError("actrailviewer did not show expected context-query events")


def main() -> int:
    args = parse_args()
    cg.require_root()
    bin_dir = ROOT / args.bin_dir
    actraild = cg.require_binary(bin_dir, "actraild")
    actrailctl = cg.require_binary(bin_dir, "actrailctl")
    actrailviewer = cg.require_binary(bin_dir, "actrailviewer")
    agent_script = GRAYLIST_DIR / "agent.py"

    with tempfile.TemporaryDirectory(prefix="actrail-control-context-query-e2e-") as raw_tmp:
        tmp = Path(raw_tmp)
        allowed = tmp / "targets" / "allowed.txt"
        gray = tmp / "targets" / "gray.txt"
        denied = tmp / "targets" / "denied.txt"
        rules = tmp / "rules.conf"
        config = tmp / "operator.conf"
        cg.write_text(allowed, "allowed\n")
        cg.write_text(gray, "gray\n")
        cg.write_text(denied, "denied\n")
        cg.write_text(
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
        cg.write_text(config, cg.operator_config(tmp, rules))

        daemon = subprocess.Popen(
            [str(actraild), "--config", str(config), "run"],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        try:
            cg.wait_for_daemon(daemon, args.daemon_ready_timeout_sec)
            cg.run_checked(
                [
                    str(actraild),
                    "--config",
                    str(config),
                    "plugin",
                    "load",
                    "--manifest",
                    str(MANIFEST),
                    "--instance",
                    INSTANCE,
                    "--grant",
                    "context-query",
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
                ],
                text=True,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            try:
                pid = cg.read_agent_pid(agent, args.agent_timeout_sec)
                cg.run_checked(
                    [
                        str(actrailctl),
                        "--config",
                        str(config),
                        "track-add",
                        "--pid",
                        str(pid),
                        "--name",
                        "control-context-query-e2e",
                    ]
                )
                if agent.stdin is None:
                    raise RuntimeError("agent stdin is not captured")
                agent.stdin.write("go\n")
                agent.stdin.flush()
                agent_output = cg.wait_for_agent_output(agent, args.agent_timeout_sec)
                if "allowed=ok" not in agent_output:
                    raise RuntimeError("agent did not confirm allow fast path")
                if "gray=ok" not in agent_output:
                    raise RuntimeError("agent did not confirm gray plugin allow")
                if "denied=permission_denied" not in agent_output:
                    raise RuntimeError("agent did not confirm deny fast path")
                viewer_output = wait_for_events(
                    actrailctl,
                    actrailviewer,
                    config,
                    args.drain_attempts,
                    args.drain_sleep_sec,
                )
                if "decision=allow" not in viewer_output or "decision=deny" not in viewer_output:
                    raise RuntimeError("viewer output missed allow/deny decisions")
                status = cg.parse_status_fields(
                    cg.run_checked(
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
                if status.get("host_grants") != "context-query":
                    raise RuntimeError(f"context-query grant missing from status\n{status}")
                if int(status.get("observed_records", "0")) != 1:
                    raise RuntimeError(f"control plugin should only see the gray decision\n{status}")
                cg.run_checked(
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
                cg.stop_process(agent)
        finally:
            cg.stop_process(daemon)

    print(f"control_context_query_plugin_instance={INSTANCE}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"control context-query e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
