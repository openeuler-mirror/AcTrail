#!/usr/bin/env python3
"""Validate graylist sync-plugin policy shape at daemon load time."""

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
HELPERS = ROOT / "tests/plugins/control-graylist/run_e2e.py"
INSTANCE = "wasm.graylist-policy"


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
    parser.add_argument("--daemon-reject-timeout-sec", type=float, default=5.0)
    return parser.parse_args()


def wait_for_rejection(process: subprocess.Popen[str], timeout_sec: float) -> str:
    deadline = time.monotonic() + timeout_sec
    lines: list[str] = []
    while time.monotonic() < deadline:
        line = helpers.read_line_until(process, process.stdout, deadline)
        if line:
            lines.append(line)
            if "daemon listening" in line:
                raise RuntimeError("daemon accepted graylist sync-plugin rule without timeout/concurrency budget")
        if process.poll() is not None:
            break
    try:
        process.wait(timeout=max(deadline - time.monotonic(), 0.1))
    except subprocess.TimeoutExpired as error:
        raise RuntimeError("daemon did not reject invalid graylist sync-plugin rule") from error
    stdout = "".join(lines)
    stderr = process.stderr.read() if process.stderr else ""
    output = stdout + stderr
    if "timeout-ms" not in output or "concurrency" not in output:
        raise RuntimeError(f"daemon rejection did not mention timeout/concurrency budget\n{output}")
    return output


def main() -> int:
    args = parse_args()
    helpers.require_root()
    bin_dir = ROOT / args.bin_dir
    actraild = helpers.require_binary(bin_dir, "actraild")

    with tempfile.TemporaryDirectory(prefix="actrail-control-policy-e2e-") as raw_tmp:
        tmp = Path(raw_tmp)
        gray = tmp / "targets" / "gray.txt"
        rules = tmp / "rules.conf"
        config = tmp / "operator.conf"
        helpers.write_text(gray, "gray\n")
        helpers.write_text(
            rules,
            f"gray-file gray sync-plugin {INSTANCE} fallback deny open {gray}\n",
        )
        helpers.write_text(config, helpers.operator_config(tmp, rules))

        daemon = subprocess.Popen(
            [str(actraild), "--config", str(config), "run"],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        try:
            wait_for_rejection(daemon, args.daemon_reject_timeout_sec)
        finally:
            helpers.stop_process(daemon)

    print("control_policy_missing_budget_rejected=ok")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"control policy e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
