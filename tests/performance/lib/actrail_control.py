"""AcTrail daemon and trace helpers for performance benchmarks."""

from __future__ import annotations

import configparser
import subprocess
import time
from pathlib import Path

from lib.paths import resolve
from lib.paths import resolve_command
from lib.trace_store import read_trace_diagnostics


def clean_operator(repo: Path, config: configparser.ConfigParser, operator: Path) -> None:
    ctl = resolve_command(repo, config["runner"]["actrailctl"])
    run_checked([str(ctl), "clean", "--config", str(operator)], repo)


def start_daemon(repo: Path, config: configparser.ConfigParser, operator: Path) -> None:
    daemon = resolve_command(repo, config["runner"]["actraild"])
    run_checked([str(daemon), "start", "--config", str(operator)], repo)
    run_checked([str(daemon), "status", "--config", str(operator)], repo)


def stop_daemon(repo: Path, config: configparser.ConfigParser, operator: Path, check: bool) -> None:
    daemon = resolve_command(repo, config["runner"]["actraild"])
    result = subprocess.run(
        [str(daemon), "stop", "--config", str(operator)],
        cwd=repo,
        text=True,
        capture_output=True,
        check=False,
    )
    if check and result.returncode != 0:
        raise RuntimeError(f"failed to stop actraild:\n{result.stdout}{result.stderr}")


def wait_for_completed_summary(
    repo: Path,
    config: configparser.ConfigParser,
    operator: Path,
    trace_id: int,
) -> str:
    viewer = resolve_command(repo, config["runner"]["actrailviewer"])
    attempts = config["runner"].getint("drain_attempts")
    sleep_seconds = config["runner"].getfloat("drain_sleep_seconds")
    summary = ""
    last_lock_error = ""
    for _ in range(attempts):
        try:
            summary = run_checked(
                [
                    str(viewer),
                    "summary",
                    "--config",
                    str(operator),
                    "--trace-id",
                    str(trace_id),
                ],
                repo,
            )
        except RuntimeError as error:
            if "database is locked" not in str(error):
                raise
            last_lock_error = str(error)
            time.sleep(sleep_seconds)
            continue
        if "state=Completed" in summary:
            return summary
        time.sleep(sleep_seconds)
    if last_lock_error and not summary:
        raise RuntimeError(last_lock_error)
    return summary


def read_diagnostics(operator: Path, trace_id: int) -> str:
    return read_trace_diagnostics(operator, trace_id)


def diagnostics_only_bootstrap_gap(diagnostics: str) -> bool:
    rows = [line for line in diagnostics.splitlines() if line.startswith("diag-")]
    return bool(rows) and all("bootstrap_gap" in row or "BootstrapGap" in row for row in rows)


def operator_config(repo: Path, config: configparser.ConfigParser, mode: str) -> Path | None:
    if mode == "baseline":
        return None
    if mode in {"daemon-idle", "observed-ebpf-core"}:
        return resolve(repo, config["runner"]["basic_operator_config"])
    if mode == "observed-ebpf-payload":
        return resolve(repo, config["runner"]["full_operator_config"])
    if mode == "observed-seccomp-agent":
        return resolve(repo, config["runner"]["llm_operator_config"])
    raise RuntimeError(f"unknown mode {mode}")


def run_checked(command: list[str], cwd: Path) -> str:
    result = subprocess.run(command, cwd=cwd, text=True, capture_output=True, check=False)
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed exit={result.returncode}: {' '.join(command)}\n"
            f"{result.stdout}{result.stderr}"
        )
    return result.stdout + result.stderr
