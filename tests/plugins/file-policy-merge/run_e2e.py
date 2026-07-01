#!/usr/bin/env python3
"""Run multi-plugin file policy merge E2E."""

from __future__ import annotations

import argparse
import importlib.util
import os
import signal
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
FIXTURE_DIR = ROOT / "tests/plugins/wasm-component-control-hostcalls"
GRAYLIST_DIR = ROOT / "tests/plugins/control-graylist"
MANIFEST = FIXTURE_DIR / "component-hostcalls.plugin.toml"
AGENT = Path(__file__).with_name("agent.py")
INSTANCE_ALLOW_HIGH = "wasm.file-policy-merge-allow-high"
INSTANCE_DENY_LOW = "wasm.file-policy-merge-deny-low"
INSTANCE_DENY_SAME = "wasm.file-policy-merge-deny-same"


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
    return parser.parse_args()


def stop_process(process: subprocess.Popen[str]) -> None:
    if process.poll() is not None:
        return
    process.send_signal(signal.SIGTERM)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def load_policy_plugin(
    actraild: Path,
    config: Path,
    instance: str,
    decision: str,
) -> None:
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
            "file-access.current-match-get",
            "--grant",
            "file-policy.rules.read",
            "--grant",
            "file-policy.rules.match-dry-run",
            "--grant",
            f"file-policy.rules.apply:kind={decision},path=/tmp/**",
            "--instance",
            instance,
        ]
    )


def run_agent(
    actrailctl: Path,
    config: Path,
    name: str,
    steps: list[tuple[str, str, Path]],
    timeout_sec: float,
) -> str:
    command = [sys.executable, str(AGENT)]
    for label, expected, path in steps:
        command.extend(["--step", f"{label}:{expected}:{path}"])
    agent = subprocess.Popen(
        command,
        text=True,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    try:
        pid = GRAYLIST.read_agent_pid(agent, timeout_sec)
        GRAYLIST.run_checked(
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
        if agent.stdin is None:
            raise RuntimeError("agent stdin is not captured")
        agent.stdin.write("go\n")
        agent.stdin.flush()
        output = GRAYLIST.wait_for_agent_output(agent, timeout_sec)
        if agent.returncode != 0:
            raise RuntimeError(f"agent failed\nstdout={output}")
        return output
    finally:
        stop_process(agent)


def main() -> int:
    args = parse_args()
    GRAYLIST.require_root()
    bin_dir = ROOT / args.bin_dir
    actraild = GRAYLIST.require_binary(bin_dir, "actraild")
    actrailctl = GRAYLIST.require_binary(bin_dir, "actrailctl")

    with tempfile.TemporaryDirectory(prefix="actrail-file-policy-merge-e2e-") as raw_tmp:
        tmp = Path(raw_tmp)
        targets = tmp / "targets"
        shared = targets / "shared.txt"
        trigger_allow = targets / "trigger-allow.txt"
        trigger_deny_low = targets / "trigger-deny-low.txt"
        trigger_deny_same = targets / "trigger-deny-same.txt"
        rules = tmp / "rules.conf"
        config = tmp / "operator.conf"
        for path in [shared, trigger_allow, trigger_deny_low, trigger_deny_same]:
            GRAYLIST.write_text(path, f"{path.name}\n")
        GRAYLIST.write_text(
            rules,
            "\n".join(
                [
                    f"merge-allow-high gray sync-plugin {INSTANCE_ALLOW_HIGH} timeout-ms 5000 concurrency 1 fallback deny open {trigger_allow}",
                    f"merge-deny-low gray sync-plugin {INSTANCE_DENY_LOW} timeout-ms 5000 concurrency 1 fallback deny open {trigger_deny_low}",
                    f"merge-deny-same gray sync-plugin {INSTANCE_DENY_SAME} timeout-ms 5000 concurrency 1 fallback deny open {trigger_deny_same}",
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
            load_policy_plugin(actraild, config, INSTANCE_ALLOW_HIGH, "allow")
            load_policy_plugin(actraild, config, INSTANCE_DENY_LOW, "deny")
            first = run_agent(
                actrailctl,
                config,
                "file-policy-merge-priority",
                [
                    ("trigger_allow", "ok", trigger_allow),
                    ("shared_after_allow", "ok", shared),
                    ("trigger_deny_low", "ok", trigger_deny_low),
                    ("shared_after_low_priority_deny", "ok", shared),
                ],
                args.agent_timeout_sec,
            )
            if "shared_after_low_priority_deny=ok" not in first:
                raise RuntimeError("priority merge did not keep high-priority allow")

            load_policy_plugin(actraild, config, INSTANCE_DENY_SAME, "deny")
            second = run_agent(
                actrailctl,
                config,
                "file-policy-merge-last-wins",
                [
                    ("trigger_deny_same", "ok", trigger_deny_same),
                    ("shared_after_same_priority_deny", "permission_denied", shared),
                ],
                args.agent_timeout_sec,
            )
            if "shared_after_same_priority_deny=permission_denied" not in second:
                raise RuntimeError("same-priority last-wins deny did not take effect")

            GRAYLIST.run_checked(
                [
                    str(actraild),
                    "--config",
                    str(config),
                    "plugin",
                    "unload",
                    "--instance",
                    INSTANCE_DENY_SAME,
                ]
            )
            third = run_agent(
                actrailctl,
                config,
                "file-policy-merge-unload-same",
                [("shared_after_unload_same", "ok", shared)],
                args.agent_timeout_sec,
            )
            if "shared_after_unload_same=ok" not in third:
                raise RuntimeError("unload did not restore high-priority allow")

            GRAYLIST.run_checked(
                [
                    str(actraild),
                    "--config",
                    str(config),
                    "plugin",
                    "unload",
                    "--instance",
                    INSTANCE_ALLOW_HIGH,
                ]
            )
            fourth = run_agent(
                actrailctl,
                config,
                "file-policy-merge-unload-allow",
                [("shared_after_unload_allow", "permission_denied", shared)],
                args.agent_timeout_sec,
            )
            if "shared_after_unload_allow=permission_denied" not in fourth:
                raise RuntimeError("unload did not expose lower-priority deny")

            print("file_policy_merge_priority=ok")
            print("file_policy_merge_last_wins=ok")
            print("file_policy_merge_unload_remerge=ok")
            return 0
        finally:
            stop_process(daemon)


if __name__ == "__main__":
    raise SystemExit(main())
