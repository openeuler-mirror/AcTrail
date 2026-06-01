"""Markdown reporting for AcTrail performance benchmarks."""

from __future__ import annotations

import datetime as dt
import platform
import random
import statistics
import sys
from dataclasses import dataclass
from pathlib import Path

from lib.statistics import distribution_stats, quantile_sorted


@dataclass(frozen=True)
class BenchRun:
    case: str
    mode: str
    iteration: int
    outer_wall_ms: float
    task_runtime_ms: float
    trace_id: int | None
    summary: str
    diagnostics: str
    workload_result: dict[str, object]


def write_report(
    repo: Path,
    output: Path,
    cases: list[str],
    modes: list[str],
    warmups: int,
    repetitions: int,
    overhead_threshold_percent: float,
    alpha: float,
    bootstrap_resamples: int,
    permutation_resamples: int,
    max_exact_permutations: int,
    random_seed: int,
    runs: list[BenchRun],
) -> None:
    output_path = resolve(repo, output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    lines = [
        "# AcTrail Performance Benchmark",
        "",
        f"- Generated: {dt.datetime.now().isoformat(timespec='seconds')}",
        f"- Host: `{platform.platform()}`",
        f"- Python: `{sys.version.split()[0]}`",
        f"- Cases: `{', '.join(cases)}`",
        f"- Modes: `{', '.join(modes)}`",
        f"- Warmups: `{warmups}`",
        f"- Repetitions: `{repetitions}`",
        f"- Distribution overhead threshold: `{overhead_threshold_percent:.3f}%`",
        f"- Alpha: `{alpha:.3f}`",
        f"- Bootstrap resamples: `{bootstrap_resamples}`",
        f"- Permutation resamples: `{permutation_resamples}`",
        "",
        "## Task Runtime Summary",
        "",
        "| Case | Mode | Runs | Median task ms | P95 task ms | Median Task Overhead vs Baseline |",
        "| --- | --- | ---: | ---: | ---: | ---: |",
    ]
    append_task_summary_rows(lines, cases, modes, runs)
    append_outer_wall_rows(lines, cases, modes, runs)
    append_distribution_rows(
        lines,
        cases,
        modes,
        overhead_threshold_percent,
        alpha,
        bootstrap_resamples,
        permutation_resamples,
        max_exact_permutations,
        random_seed,
        runs,
    )
    append_raw_runs(lines, runs)
    output_path.write_text("\n".join(lines), encoding="utf-8")


def append_task_summary_rows(
    lines: list[str],
    cases: list[str],
    modes: list[str],
    runs: list[BenchRun],
) -> None:
    for case in cases:
        baseline = median_for(runs, case, "baseline", "task")
        for mode in modes:
            values = runtime_values(runs, case, mode, "task")
            if not values:
                continue
            overhead = "n/a"
            if baseline is not None and mode != "baseline":
                overhead = f"{((statistics.median(values) - baseline) / baseline) * 100:.2f}%"
            lines.append(
                "| "
                + " | ".join(
                    [
                        case,
                        mode,
                        str(len(values)),
                        f"{statistics.median(values):.3f}",
                        f"{percentile(values, 95):.3f}",
                        overhead,
                    ]
                )
                + " |"
            )


def append_outer_wall_rows(
    lines: list[str],
    cases: list[str],
    modes: list[str],
    runs: list[BenchRun],
) -> None:
    lines.extend(
        [
            "",
            "## Outer Command Wall-Clock",
            "",
            "| Case | Mode | Runs | Median outer ms | Median outer-task ms |",
            "| --- | --- | ---: | ---: | ---: |",
        ]
    )
    for case in cases:
        for mode in modes:
            outer_values = runtime_values(runs, case, mode, "outer")
            task_values = runtime_values(runs, case, mode, "task")
            if not outer_values or not task_values:
                continue
            gaps = [
                run.outer_wall_ms - run.task_runtime_ms
                for run in runs
                if run.case == case and run.mode == mode
            ]
            lines.append(
                "| "
                + " | ".join(
                    [
                        case,
                        mode,
                        str(len(outer_values)),
                        f"{statistics.median(outer_values):.3f}",
                        f"{statistics.median(gaps):.3f}",
                    ]
                )
                + " |"
            )


def append_distribution_rows(
    lines: list[str],
    cases: list[str],
    modes: list[str],
    overhead_threshold_percent: float,
    alpha: float,
    bootstrap_resamples: int,
    permutation_resamples: int,
    max_exact_permutations: int,
    random_seed: int,
    runs: list[BenchRun],
) -> None:
    lines.extend(
        [
            "",
            "## Distribution-Level 5% Overhead Assessment",
            "",
            "| Case | Mode | Baseline n | Observed n | HL Overhead | CI | KS p(same distribution) | MW p(> threshold) | Decision |",
            "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |",
        ]
    )
    rng = random.Random(random_seed)
    for case in cases:
        baseline = runtime_values(runs, case, "baseline", "task")
        if not baseline:
            continue
        for mode in modes:
            if mode == "baseline":
                continue
            observed = runtime_values(runs, case, mode, "task")
            if not observed:
                continue
            stats = distribution_stats(
                baseline,
                observed,
                overhead_threshold_percent,
                alpha,
                bootstrap_resamples,
                permutation_resamples,
                max_exact_permutations,
                rng,
            )
            lines.append(
                "| "
                + " | ".join(
                    [
                        case,
                        mode,
                        str(len(baseline)),
                        str(len(observed)),
                        f"{stats.hl_overhead_percent:.2f}%",
                        f"[{stats.ci_low_percent:.2f}%, {stats.ci_high_percent:.2f}%]",
                        f"{stats.ks_p_value:.4f}",
                        f"{stats.mw_p_value:.4f}",
                        stats.decision,
                    ]
                )
                + " |"
            )


def append_raw_runs(lines: list[str], runs: list[BenchRun]) -> None:
    lines.extend(["", "## Raw Runs", ""])
    for run in runs:
        trace = f"trace-{run.trace_id}" if run.trace_id is not None else "n/a"
        lines.append(
            f"- case={run.case} mode={run.mode} iteration={run.iteration} "
            f"task_runtime_ms={run.task_runtime_ms:.3f} "
            f"outer_wall_ms={run.outer_wall_ms:.3f} trace={trace} "
            f"diagnostics={diagnostic_kinds(run.diagnostics)} workload={run.workload_result}"
        )
    lines.extend(
        [
            "",
            "## Interpretation Notes",
            "",
            "- `baseline` runs the workload without AcTrail.",
            "- `daemon-idle` keeps `actraild` running but does not attach the workload.",
            "- `observed-ebpf-core` uses `actrailctl launch` with eBPF process/file/network observation.",
            "- `observed-ebpf-payload` adds stdio payload, socket plaintext payload, HTTP/1.1 semantics, and resource sampling; it does not enable seccomp exec/fork or TLS LLM payload capture.",
            "- `observed-seccomp-agent` enables process/network observation plus seccomp exec/fork/clone observation and agent invocation semantics; it is the mode intended for real external LLM CLI wall-clock tests.",
            "- `task_runtime_ms` is measured by `tests/performance/lib/timing_wrapper.py` immediately around the target workload/CLI command in both baseline and observed modes.",
            "- `outer_wall_ms` includes benchmark runner and `actrailctl launch` wrapper time. It is reported separately and is not used for the 5% overhead assessment.",
            "- `HL Overhead` is the Hodges-Lehmann estimator over all observed/baseline pairwise runtime ratios, so it compares distributions rather than matching iteration numbers.",
            "- `CI` is a bootstrap confidence interval for the distribution-level pairwise ratio overhead.",
            "- `KS p(same distribution)` is a two-sample permutation Kolmogorov-Smirnov p-value. A large p-value does not prove equality; it only means this sample is insufficient to reject same-distribution behavior.",
            "- `MW p(> threshold)` is a one-sided Mann-Whitney permutation p-value after scaling the baseline sample by the configured threshold.",
            "- A benchmark result is invalid if any run fails, times out, or produces a non-BootstrapGap degraded trace.",
            "",
        ]
    )


def diagnostic_kinds(diagnostics: str) -> str:
    kinds: list[str] = []
    for line in diagnostics.splitlines():
        if line.startswith("diag-"):
            fields = line.split()
            if len(fields) >= 3:
                kinds.append(fields[2])
    return ",".join(kinds) if kinds else "none"


def runtime_values(runs: list[BenchRun], case: str, mode: str, kind: str) -> list[float]:
    if kind == "task":
        return [run.task_runtime_ms for run in runs if run.case == case and run.mode == mode]
    if kind == "outer":
        return [run.outer_wall_ms for run in runs if run.case == case and run.mode == mode]
    raise RuntimeError(f"unknown runtime kind {kind}")


def median_for(runs: list[BenchRun], case: str, mode: str, kind: str) -> float | None:
    values = runtime_values(runs, case, mode, kind)
    if not values:
        return None
    return statistics.median(values)


def percentile(values: list[float], percentile_value: float) -> float:
    return quantile_sorted(sorted(values), percentile_value / 100.0)


def resolve(repo: Path, path: Path) -> Path:
    if path.is_absolute():
        return path
    return repo / path
