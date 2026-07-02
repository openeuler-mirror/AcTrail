#!/usr/bin/env python3
"""Run AcTrail local performance benchmarks and write a Markdown report."""

from __future__ import annotations

import argparse
import configparser
import json
import re
import shlex
import subprocess
import sys
import time
from pathlib import Path

from lib.actrail_control import clean_operator
from lib.actrail_control import diagnostics_only_bootstrap_gap
from lib.actrail_control import operator_config
from lib.actrail_control import read_diagnostics
from lib.actrail_control import start_daemon
from lib.actrail_control import stop_daemon
from lib.actrail_control import wait_for_completed_summary
from lib.paths import require_file
from lib.paths import resolve
from lib.paths import resolve_command
from report import BenchRun, write_report


TRACE_RE = re.compile(r"trace trace-(\d+) entered Active")
TASK_TIMING_MARKER = "BENCHMARK_TASK_TIMING "


def main() -> int:
    args = parse_args()
    repo = Path.cwd()
    config = read_config(args.benchmark_config)
    cases = selected_values(args.case, config["runner"]["cases"])
    modes = selected_values(args.mode, config["runner"]["modes"])
    warmups = args.warmups if args.warmups is not None else config["runner"].getint("warmups")
    repetitions = (
        args.repetitions if args.repetitions is not None else config["runner"].getint("repetitions")
    )
    output = Path(args.output or config["runner"]["output_markdown"])
    overhead_threshold_percent = (
        args.overhead_threshold_percent
        if args.overhead_threshold_percent is not None
        else config["statistics"].getfloat("overhead_threshold_percent")
    )
    alpha = args.alpha if args.alpha is not None else config["statistics"].getfloat("alpha")
    bootstrap_resamples = (
        args.bootstrap_resamples
        if args.bootstrap_resamples is not None
        else config["statistics"].getint("bootstrap_resamples")
    )
    permutation_resamples = (
        args.permutation_resamples
        if args.permutation_resamples is not None
        else config["statistics"].getint("permutation_resamples")
    )
    max_exact_permutations = (
        args.max_exact_permutations
        if args.max_exact_permutations is not None
        else config["statistics"].getint("max_exact_permutations")
    )
    random_seed = (
        args.random_seed if args.random_seed is not None else config["statistics"].getint("random_seed")
    )
    validate_environment(repo, config, modes)
    runs: list[BenchRun] = []
    for case in cases:
        for iteration in range(warmups + repetitions):
            record = iteration >= warmups
            visible_iteration = iteration - warmups + 1
            for mode in modes:
                if record:
                    print(f"case={case} mode={mode} iteration={visible_iteration}", flush=True)
                else:
                    print(f"case={case} mode={mode} warmup={iteration + 1}", flush=True)
                run = run_one(repo, config, case, mode, visible_iteration)
                if record:
                    runs.append(run)
    write_report(
        repo,
        output,
        cases,
        modes,
        warmups,
        repetitions,
        overhead_threshold_percent,
        alpha,
        bootstrap_resamples,
        permutation_resamples,
        max_exact_permutations,
        random_seed,
        runs,
    )
    print(f"benchmark_report={output}")
    return 0


def parse_args() -> argparse.Namespace:
    default_config = Path(__file__).resolve().parent / "benchmark.conf"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--benchmark-config", default=str(default_config))
    parser.add_argument("--case", default="all", help="file, process, http, agent, or all")
    parser.add_argument(
        "--mode",
        default="all",
        help=(
            "baseline, daemon-idle, observed-ebpf-core, "
            "observed-ebpf-payload, observed-seccomp-agent, or all"
        ),
    )
    parser.add_argument("--repetitions", type=int)
    parser.add_argument("--warmups", type=int)
    parser.add_argument("--output")
    parser.add_argument("--overhead-threshold-percent", type=float)
    parser.add_argument("--alpha", type=float)
    parser.add_argument("--bootstrap-resamples", type=int)
    parser.add_argument("--permutation-resamples", type=int)
    parser.add_argument("--max-exact-permutations", type=int)
    parser.add_argument("--random-seed", type=int)
    return parser.parse_args()


def read_config(path: str) -> configparser.ConfigParser:
    config = configparser.ConfigParser()
    loaded = config.read(path, encoding="utf-8")
    if not loaded:
        raise RuntimeError(f"benchmark config not found: {path}")
    setattr(config, "_source_path", Path(path).resolve())
    return config


def selected_values(selection: str, configured_csv: str) -> list[str]:
    configured = [value.strip() for value in configured_csv.split(",") if value.strip()]
    if selection == "all":
        return configured
    values = [value.strip() for value in selection.split(",") if value.strip()]
    unknown = [value for value in values if value not in configured]
    if unknown:
        raise RuntimeError(f"unknown selection {unknown}; allowed={configured}")
    return values


def validate_environment(repo: Path, config: configparser.ConfigParser, modes: list[str]) -> None:
    require_file(resolve_command(repo, config["runner"]["python"]), executable=False)
    require_file(resolve(repo, config["runner"]["workload_script"]), executable=False)
    require_file(resolve(repo, config["runner"]["timing_wrapper_script"]), executable=False)
    if any(mode != "baseline" for mode in modes):
        for key in ["actraild", "actrailctl", "actrailviewer"]:
            require_file(resolve_command(repo, config["runner"][key]), executable=False)
        for mode in modes:
            operator = operator_config(repo, config, mode)
            if operator is not None:
                require_file(operator, executable=False)


def run_one(
    repo: Path,
    config: configparser.ConfigParser,
    case: str,
    mode: str,
    iteration: int,
) -> BenchRun:
    operator = operator_config(repo, config, mode)
    if operator is None:
        return run_workload_direct(repo, config, case, mode, iteration)
    stop_daemon(repo, config, operator, check=False)
    clean_operator(repo, config, operator)
    start_daemon(repo, config, operator)
    try:
        if mode == "daemon-idle":
            return run_workload_direct(repo, config, case, mode, iteration)
        run = run_workload_observed(repo, config, case, mode, iteration, operator)
        return run
    finally:
        stop_daemon(repo, config, operator, check=True)


def run_workload_direct(
    repo: Path,
    config: configparser.ConfigParser,
    case: str,
    mode: str,
    iteration: int,
) -> BenchRun:
    target_command = case_command(repo, config, case)
    command = timed_case_command(repo, config, target_command)
    outer_wall_ms, output = run_timed(command, repo, case_timeout(config, case))
    task_runtime_ms = parse_task_runtime_ms(output)
    workload = parse_case_result(config, case, target_command, output)
    return BenchRun(
        case,
        mode,
        iteration,
        outer_wall_ms,
        task_runtime_ms,
        None,
        "",
        "",
        workload,
    )


def run_workload_observed(
    repo: Path,
    config: configparser.ConfigParser,
    case: str,
    mode: str,
    iteration: int,
    operator: Path,
) -> BenchRun:
    actrailctl = resolve_command(repo, config["runner"]["actrailctl"])
    command = [
        str(actrailctl),
        "launch",
        "--config",
        str(operator),
        "--name",
        f"perf-{case}-{mode}-{iteration}",
        "--",
    ]
    target_command = case_command(repo, config, case)
    command.extend(timed_case_command(repo, config, target_command))
    outer_wall_ms, output = run_timed(command, repo, case_timeout(config, case))
    trace_id = parse_trace_id(output)
    task_runtime_ms = parse_task_runtime_ms(output)
    workload = parse_case_result(config, case, target_command, output)
    summary = wait_for_completed_summary(repo, config, operator, trace_id)
    diagnostics = read_diagnostics(operator, trace_id)
    if "state=Exited" not in summary:
        raise RuntimeError(f"trace trace-{trace_id} did not exit:\n{summary}")
    if "health=Degraded" in summary and not diagnostics_only_bootstrap_gap(diagnostics):
        raise RuntimeError(f"trace trace-{trace_id} degraded:\n{summary}\n{diagnostics}")
    return BenchRun(
        case,
        mode,
        iteration,
        outer_wall_ms,
        task_runtime_ms,
        trace_id,
        summary,
        diagnostics,
        workload,
    )


def case_command(repo: Path, config: configparser.ConfigParser, case: str) -> list[str]:
    if config.has_section(case) and "command_argv" in config[case]:
        return shlex.split(config[case]["command_argv"])
    return workload_command(repo, config, case)


def workload_command(repo: Path, config: configparser.ConfigParser, case: str) -> list[str]:
    return [
        str(resolve_command(repo, config["runner"]["python"])),
        str(resolve(repo, config["runner"]["workload_script"])),
        "--case",
        case,
        "--config",
        str(config_path(config)),
    ]


def timed_case_command(
    repo: Path,
    config: configparser.ConfigParser,
    target_command: list[str],
) -> list[str]:
    return [
        str(resolve_command(repo, config["runner"]["python"])),
        str(resolve(repo, config["runner"]["timing_wrapper_script"])),
        "--",
        *target_command,
    ]


def case_timeout(config: configparser.ConfigParser, case: str) -> float:
    if config.has_section(case) and "timeout_seconds" in config[case]:
        return config[case].getfloat("timeout_seconds")
    return config["runner"].getfloat("timeout_seconds")


def parse_case_result(
    config: configparser.ConfigParser,
    case: str,
    command: list[str],
    output: str,
) -> dict[str, object]:
    if config.has_section(case) and "command_argv" in config[case]:
        output_bytes = len(target_output(output).encode("utf-8"))
        min_output = config[case].getint("min_combined_output_bytes")
        if output_bytes < min_output:
            raise RuntimeError(f"{case} command produced less than {min_output} bytes")
        return {
            "case": case,
            "command": command[0],
            "combined_output_bytes": output_bytes,
        }
    return parse_workload_result(output)


def parse_task_runtime_ms(output: str) -> float:
    for line in reversed(output.splitlines()):
        if line.startswith(TASK_TIMING_MARKER):
            payload = line.split(" ", 1)[1]
            try:
                value = json.loads(payload).get("task_runtime_ms")
            except json.JSONDecodeError as error:
                raise RuntimeError(f"invalid task runtime marker: {line}") from error
            if not isinstance(value, (int, float)):
                raise RuntimeError(f"task runtime marker missing numeric task_runtime_ms: {line}")
            runtime_ms = float(value)
            if runtime_ms <= 0:
                raise RuntimeError(f"task runtime marker must be positive: {line}")
            return runtime_ms
    raise RuntimeError(f"task runtime marker missing:\n{output}")


def target_output(output: str) -> str:
    lines = []
    for line in output.splitlines():
        if line.startswith(TASK_TIMING_MARKER):
            continue
        if TRACE_RE.search(line):
            continue
        lines.append(line)
    return "\n".join(lines)


def config_path(config: configparser.ConfigParser) -> Path:
    path = getattr(config, "_source_path", None)
    if not path:
        raise RuntimeError("benchmark config source path missing")
    return Path(path)


def run_timed(command: list[str], cwd: Path, timeout: float) -> tuple[float, str]:
    start_ns = time.perf_counter_ns()
    result = subprocess.run(
        command,
        cwd=cwd,
        text=True,
        capture_output=True,
        timeout=timeout,
        check=False,
    )
    elapsed_ms = (time.perf_counter_ns() - start_ns) / 1_000_000
    output = result.stdout + result.stderr
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed exit={result.returncode}: {' '.join(command)}\n{output}"
        )
    return elapsed_ms, output


def parse_workload_result(output: str) -> dict[str, object]:
    for line in reversed(output.splitlines()):
        if line.startswith("BENCHMARK_RESULT "):
            payload = line.split(" ", 1)[1]
            return json.loads(payload)
    raise RuntimeError(f"workload result marker missing:\n{output}")


def parse_trace_id(output: str) -> int:
    match = TRACE_RE.search(output)
    if not match:
        raise RuntimeError(f"trace id missing from launch output:\n{output}")
    return int(match.group(1))


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"performance benchmark failed: {error}", file=sys.stderr)
        raise SystemExit(1)
