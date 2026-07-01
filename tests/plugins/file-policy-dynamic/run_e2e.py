#!/usr/bin/env python3
"""Run the dynamic file policy plugin E2E."""

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
GRAYLIST_DIR = ROOT / "tests/plugins/control-graylist"
MERGE_AGENT = ROOT / "tests/plugins/file-policy-merge/agent.py"
MANIFEST = ROOT / "examples/plugins/wit-component/file-policy-dynamic/plugin.toml"
INSTANCE = "wasm.file-policy-dynamic"


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


def path_hash_even(path: Path) -> bool:
    value = 0
    for byte in str(path).encode("utf-8"):
        value = ((value * 131) + byte) & ((1 << 64) - 1)
    return value % 2 == 0


def find_parity_path(directory: Path, prefix: str, even: bool) -> Path:
    for index in range(100):
        path = directory / f"{prefix}-{index}.txt"
        if path_hash_even(path) == even:
            return path
    raise RuntimeError(f"failed to find {'even' if even else 'odd'} hash path")


def plugin_cmd(actraild: Path, config: Path, *argv: str) -> str:
    return GRAYLIST.run_checked(
        [
            str(actraild),
            "--config",
            str(config),
            "plugin",
            "cmd",
            "--instance",
            INSTANCE,
            "--",
            *argv,
        ]
    )


def run_agent(
    actrailctl: Path,
    config: Path,
    name: str,
    steps: list[tuple[str, str, Path]],
    timeout_sec: float,
) -> str:
    command = [sys.executable, str(MERGE_AGENT)]
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


def rule_id_for(output: str, decision: str, path: Path) -> str:
    expected_path = str(path)
    for line in output.splitlines():
        fields = line.split()
        if len(fields) >= 3 and fields[1] == decision and fields[2] == expected_path:
            return fields[0]
    raise RuntimeError(f"rule id not found for {decision} {path}\n{output}")


def main() -> int:
    args = parse_args()
    GRAYLIST.require_root()
    bin_dir = ROOT / args.bin_dir
    actraild = GRAYLIST.require_binary(bin_dir, "actraild")
    actrailctl = GRAYLIST.require_binary(bin_dir, "actrailctl")

    with tempfile.TemporaryDirectory(prefix="actrail-file-policy-dynamic-e2e-") as raw_tmp:
        tmp = Path(raw_tmp)
        targets = tmp / "targets"
        allow_path = targets / "allow-by-plugin.txt"
        deny_path = targets / "deny-by-plugin.txt"
        gray_even = find_parity_path(targets, "gray-even", True)
        gray_odd = find_parity_path(targets, "gray-odd", False)
        for path in [allow_path, deny_path, gray_even, gray_odd]:
            GRAYLIST.write_text(path, f"{path.name}\n")
        rules = tmp / "rules.conf"
        config = tmp / "operator.conf"
        GRAYLIST.write_text(
            rules,
            "\n".join(
                [
                    f"gray-even gray sync-plugin {INSTANCE} timeout-ms 5000 concurrency 1 fallback deny open {gray_even}",
                    f"gray-odd gray sync-plugin {INSTANCE} timeout-ms 5000 concurrency 1 fallback deny open {gray_odd}",
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
                    "file-policy.rules.read",
                    "--grant",
                    "file-policy.rules.match-dry-run",
                    "--grant",
                    "file-policy.rules.apply:kind=allow,path=/tmp/**",
                    "--grant",
                    "file-policy.rules.apply:kind=deny,path=/tmp/**",
                    "--grant",
                    "file-policy.rules.apply:kind=gray,path=/tmp/**",
                    "--instance",
                    INSTANCE,
                ]
            )

            upsert_allow = plugin_cmd(
                actraild,
                config,
                "rule",
                "upsert",
                "allow",
                str(allow_path),
                "--priority",
                "10",
            )
            if "accepted" not in upsert_allow:
                raise RuntimeError(f"allow upsert did not succeed\n{upsert_allow}")
            dry_allow = plugin_cmd(actraild, config, "rule", "dry-run", str(allow_path))
            if "matched=true" not in dry_allow or "decision=allow" not in dry_allow:
                raise RuntimeError(f"allow dry-run missed rule\n{dry_allow}")
            run_agent(
                actrailctl,
                config,
                "dynamic-policy-allow",
                [("allow_by_plugin", "ok", allow_path)],
                args.agent_timeout_sec,
            )

            upsert_deny = plugin_cmd(
                actraild,
                config,
                "rule",
                "upsert",
                "deny",
                str(deny_path),
                "--priority",
                "10",
            )
            if "accepted" not in upsert_deny:
                raise RuntimeError(f"deny upsert did not succeed\n{upsert_deny}")
            run_agent(
                actrailctl,
                config,
                "dynamic-policy-deny",
                [("deny_by_plugin", "permission_denied", deny_path)],
                args.agent_timeout_sec,
            )

            listed = plugin_cmd(actraild, config, "rule", "list")
            deny_rule_id = rule_id_for(listed, "deny", deny_path)
            delete_deny = plugin_cmd(actraild, config, "rule", "delete", deny_rule_id)
            if "deleted=" not in delete_deny:
                raise RuntimeError(f"deny delete did not succeed\n{delete_deny}")
            dry_deleted = plugin_cmd(actraild, config, "rule", "dry-run", str(deny_path))
            if "matched=false" not in dry_deleted:
                raise RuntimeError(f"deleted rule still matched\n{dry_deleted}")

            run_agent(
                actrailctl,
                config,
                "dynamic-policy-gray-hash",
                [
                    ("gray_even", "ok", gray_even),
                    ("gray_odd", "permission_denied", gray_odd),
                ],
                args.agent_timeout_sec,
            )
            print("file_policy_dynamic_cmd_upsert_allow=ok")
            print("file_policy_dynamic_cmd_upsert_deny=ok")
            print("file_policy_dynamic_cmd_list_delete=ok")
            print("file_policy_dynamic_gray_hash=ok")
            return 0
        finally:
            stop_process(daemon)


if __name__ == "__main__":
    raise SystemExit(main())
