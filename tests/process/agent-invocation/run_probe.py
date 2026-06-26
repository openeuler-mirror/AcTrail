#!/usr/bin/env python3
"""Reusable probes for the xiaoO -> Claude agent invocation case."""

from __future__ import annotations

import argparse
import glob
import shlex
import subprocess
import sys
import time
from pathlib import Path

import run_e2e


def main() -> int:
    args = parse_args()
    config = Path(args.config)
    workload_path = Path(args.workload_config)
    workload = run_e2e.read_config(workload_path)
    prompt = run_e2e.render_workload_prompt(workload, workload_path)
    bin_dir = Path(args.bin_dir)
    mode = args.mode
    if mode == "bare-xiaoo":
        run_bare_xiaoo(workload, prompt)
    elif mode == "direct-claude":
        run_direct_claude(bin_dir, config, workload)
    elif mode == "strace-xiaoo":
        run_strace_xiaoo(workload, prompt)
    else:
        raise RuntimeError(f"unknown probe mode: {mode}")
    return 0


def parse_args() -> argparse.Namespace:
    test_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=["bare-xiaoo", "direct-claude", "strace-xiaoo"])
    parser.add_argument("--bin-dir", default="target/release")
    parser.add_argument("--config", default=str(test_dir / "operator.conf"))
    parser.add_argument("--workload-config", default=str(test_dir / "workload.conf"))
    return parser.parse_args()


def run_bare_xiaoo(workload: dict[str, str], prompt: str) -> None:
    agent_command = run_e2e.resolve_agent_command(run_e2e.required(workload, "agent_command"))
    command = [agent_command, "--cli", "run", "-p", prompt]
    result, elapsed = run_command(command, float(run_e2e.required(workload, "launch_timeout_seconds")))
    print(result.stdout, end="")
    if result.stderr:
        print(result.stderr, end="", file=sys.stderr)
    print(f"probe=bare-xiaoo elapsed_seconds={elapsed:.1f}")
    require_success(result)


def run_direct_claude(bin_dir: Path, config: Path, workload: dict[str, str]) -> None:
    run_e2e.require_root()
    actraild = run_e2e.require_binary(bin_dir, "actraild")
    actrailctl = run_e2e.require_binary(bin_dir, "actrailctl")
    run_e2e.require_command("claude")
    run_e2e.clean_configured_paths(actrailctl, config)
    daemon = subprocess.Popen(
        [str(actraild), "--config", str(config), "run"],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    try:
        run_e2e.wait_for_daemon(
            daemon,
            float(run_e2e.required(workload, "daemon_ready_timeout_seconds")),
        )
        command = [
            str(actrailctl),
            "--config",
            str(config),
            "launch",
            "--name",
            "direct-claude-sentinel",
            "--",
            "claude",
            *shlex.split(workload.get("claude_extra_args", "")),
            "-p",
            run_e2e.required(workload, "direct_claude_prompt"),
        ]
        result, elapsed = run_command(
            command,
            float(run_e2e.required(workload, "launch_timeout_seconds")),
        )
        print(result.stdout, end="")
        if result.stderr:
            print(result.stderr, end="", file=sys.stderr)
        print(f"probe=direct-claude elapsed_seconds={elapsed:.1f}")
        require_success(result)
    finally:
        run_e2e.stop_process(
            daemon,
            float(run_e2e.required(workload, "daemon_stop_timeout_seconds")),
        )


def run_strace_xiaoo(workload: dict[str, str], prompt: str) -> None:
    run_e2e.require_command("strace")
    run_e2e.require_command("timeout")
    agent_command = run_e2e.resolve_agent_command(run_e2e.required(workload, "agent_command"))
    prefix = run_e2e.required(workload, "strace_output_prefix")
    command = [
        "strace",
        "-ff",
        "-e",
        "trace=clone,clone3,fork,vfork,execve",
        "-o",
        prefix,
        "timeout",
        run_e2e.required(workload, "launch_timeout_seconds"),
        agent_command,
        "--cli",
        "run",
        "-p",
        prompt,
    ]
    result, elapsed = run_command(
        command,
        float(run_e2e.required(workload, "launch_timeout_seconds"))
        + float(run_e2e.required(workload, "strace_outer_timeout_grace_seconds")),
    )
    print(result.stdout, end="")
    if result.stderr:
        print(result.stderr, end="", file=sys.stderr)
    print(f"probe=strace-xiaoo elapsed_seconds={elapsed:.1f}")
    print_strace_summary(prefix)
    require_success(result)


def run_command(command: list[str], timeout_sec: float) -> tuple[subprocess.CompletedProcess[str], float]:
    started = time.monotonic()
    result = subprocess.run(
        command,
        text=True,
        capture_output=True,
        timeout=timeout_sec,
        check=False,
    )
    return result, time.monotonic() - started


def require_success(result: subprocess.CompletedProcess[str]) -> None:
    if result.returncode != 0:
        raise RuntimeError(f"probe command failed with status {result.returncode}")


def print_strace_summary(prefix: str) -> None:
    counts = {"clone": 0, "clone3": 0, "fork": 0, "vfork": 0, "execve": 0}
    for path in glob.glob(f"{prefix}.*"):
        for line in Path(path).read_text(encoding="utf-8", errors="replace").splitlines():
            for syscall in counts:
                if line.startswith(f"{syscall}("):
                    counts[syscall] += 1
    rendered = " ".join(f"{name}={count}" for name, count in counts.items())
    print(f"strace_summary {rendered}")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"agent invocation probe failed: {error}", file=sys.stderr)
        raise SystemExit(1)
