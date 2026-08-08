#!/usr/bin/env python3
"""Benchmark tls-probe-point-finder fast vs detect with alternating A/B order."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import statistics
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Sample:
    wall_seconds: float
    user_seconds: float
    sys_seconds: float
    maxrss_kb: int
    minflt: int
    majflt: int

    @property
    def wall_ms(self) -> float:
        return self.wall_seconds * 1000

    @property
    def cpu_ms(self) -> float:
        return (self.user_seconds + self.sys_seconds) * 1000

    @property
    def cpu_percent(self) -> float:
        if self.wall_seconds <= 0:
            return float("nan")
        return (self.user_seconds + self.sys_seconds) / self.wall_seconds * 100

    @property
    def maxrss_mb(self) -> float:
        return self.maxrss_kb / 1024


def default_target() -> str:
    configured = os.environ.get("CODEX_E2E_BINARY")
    if configured:
        return configured
    found = shutil.which("codex")
    if found:
        return found
    raise SystemExit("codex not found; pass --target explicitly")


def resolve_target(value: str) -> str:
    candidate = Path(value).expanduser()
    if candidate.is_file():
        return str(candidate)
    resolved = shutil.which(value)
    if resolved:
        return resolved
    raise SystemExit(f"binary not found and not resolvable on PATH: {value}")


def measure(finder: Path, target: str, mode: str, scan: str | None) -> Sample:
    command = [str(finder), mode]
    if scan is not None:
        command += ["--scan", scan]
    command += ["--provider", "auto", "--source", "executable", target]
    started = time.perf_counter()
    process = subprocess.Popen(
        command,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )
    _, status, usage = os.wait4(process.pid, 0)
    wall_seconds = time.perf_counter() - started
    stderr = process.stderr.read() if process.stderr else ""
    returncode = os.waitstatus_to_exitcode(status)
    if returncode != 0:
        detail = stderr.strip()[-2000:]
        raise RuntimeError(
            f"{mode} exited with {returncode}: {' '.join(command)}\n{detail}"
        )
    return Sample(
        wall_seconds=wall_seconds,
        user_seconds=usage.ru_utime,
        sys_seconds=usage.ru_stime,
        maxrss_kb=usage.ru_maxrss,
        minflt=usage.ru_minflt,
        majflt=usage.ru_majflt,
    )


def summarize(values: list[float]) -> dict[str, float]:
    ordered = sorted(values)
    return {
        "mean": statistics.mean(values),
        "median": statistics.median(values),
        "p95": ordered[int(len(ordered) * 0.95) - 1],
        "min": ordered[0],
        "max": ordered[-1],
    }


def format_summary(
    fast: dict[str, float],
    detect: dict[str, float],
    unit: str,
) -> str:
    lines = []
    for key in ("mean", "median", "p95", "min", "max"):
        fast_value = fast[key]
        detect_value = detect[key]
        ratio = detect_value / fast_value if fast_value else float("nan")
        lines.append(
            f"{key:>7} fast={fast_value:>10.3f} detect={detect_value:>10.3f} "
            f"delta={detect_value - fast_value:>+10.3f} ratio={ratio:>6.2f}x ({unit})"
        )
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--finder",
        type=Path,
        default=Path(__file__).resolve().parents[2]
        / "target/release/tls-probe-point-finder",
    )
    parser.add_argument(
        "--binary",
        "--target",
        dest="target",
        default=default_target(),
        help="ELF binary to probe (default: codex from PATH/CODEX_E2E_BINARY)",
    )
    parser.add_argument("--rounds", type=int, default=20)
    parser.add_argument(
        "--scan",
        choices=("full", "low"),
        help="finder memory scan strategy (default: omit --scan, follows the "
        "finder binary default)",
    )
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    finder = args.finder.resolve()
    if not finder.is_file():
        raise SystemExit(f"finder binary not found: {finder}")
    target = resolve_target(args.target)

    try:
        measure(finder, target, "fast", args.scan)
        measure(finder, target, "detect", args.scan)
    except RuntimeError as error:
        raise SystemExit(f"benchmark failed: {error}") from error

    fast_wall: list[float] = []
    detect_wall: list[float] = []
    fast_cpu: list[float] = []
    detect_cpu: list[float] = []
    fast_rss: list[float] = []
    detect_rss: list[float] = []
    rows: list[dict[str, object]] = []

    if not args.json:
        scan_label = (
            args.scan if args.scan else "finder-default (--scan omitted)"
        )
        print(
            f"benchmark rounds={args.rounds} scan={scan_label} "
            f"finder={finder} target={target}\n"
        )
        print(
            f"{'round':>5} {'order':>8} {'fast_wall':>10} {'fast_cpu':>9} "
            f"{'fast_rss':>9} {'detect_wall':>12} {'detect_cpu':>11} "
            f"{'detect_rss':>11} {'delta_wall':>11} {'delta_rss':>10}"
        )

    for index in range(args.rounds):
        if index % 2 == 0:
            first, second = "fast", "detect"
            fast_sample = measure(finder, target, first, args.scan)
            detect_sample = measure(finder, target, second, args.scan)
        else:
            first, second = "detect", "fast"
            detect_sample = measure(finder, target, first, args.scan)
            fast_sample = measure(finder, target, second, args.scan)

        fast_wall.append(fast_sample.wall_ms)
        detect_wall.append(detect_sample.wall_ms)
        fast_cpu.append(fast_sample.cpu_ms)
        detect_cpu.append(detect_sample.cpu_ms)
        fast_rss.append(fast_sample.maxrss_mb)
        detect_rss.append(detect_sample.maxrss_mb)
        rows.append(
            {
                "round": index + 1,
                "order": f"{first}/{second}",
                "fast_wall_ms": fast_sample.wall_ms,
                "fast_cpu_ms": fast_sample.cpu_ms,
                "fast_maxrss_mb": fast_sample.maxrss_mb,
                "fast_minflt": fast_sample.minflt,
                "fast_majflt": fast_sample.majflt,
                "detect_wall_ms": detect_sample.wall_ms,
                "detect_cpu_ms": detect_sample.cpu_ms,
                "detect_maxrss_mb": detect_sample.maxrss_mb,
                "detect_minflt": detect_sample.minflt,
                "detect_majflt": detect_sample.majflt,
            }
        )
        if not args.json:
            print(
                f"{index + 1:>5} {first + '/' + second:>8} "
                f"{fast_sample.wall_ms:>10.3f} {fast_sample.cpu_ms:>9.3f} "
                f"{fast_sample.maxrss_mb:>9.2f} {detect_sample.wall_ms:>12.3f} "
                f"{detect_sample.cpu_ms:>11.3f} {detect_sample.maxrss_mb:>11.2f} "
                f"{detect_sample.wall_ms - fast_sample.wall_ms:>+11.3f} "
                f"{detect_sample.maxrss_mb - fast_sample.maxrss_mb:>+10.2f}",
                flush=True,
            )

    fast_wall_stats = summarize(fast_wall)
    detect_wall_stats = summarize(detect_wall)
    fast_cpu_stats = summarize(fast_cpu)
    detect_cpu_stats = summarize(detect_cpu)
    fast_rss_stats = summarize(fast_rss)
    detect_rss_stats = summarize(detect_rss)

    if args.json:
        print(
            json.dumps(
                {
                    "rounds": args.rounds,
                    "finder": str(finder),
                    "target": target,
                    "samples": rows,
                    "summary": {
                        "wall_ms": {
                            "fast": fast_wall_stats,
                            "detect": detect_wall_stats,
                        },
                        "cpu_ms": {
                            "fast": fast_cpu_stats,
                            "detect": detect_cpu_stats,
                        },
                        "maxrss_mb": {
                            "fast": fast_rss_stats,
                            "detect": detect_rss_stats,
                        },
                    },
                },
                indent=2,
            )
        )
        return

    print("\nwall_ms summary")
    print(format_summary(fast_wall_stats, detect_wall_stats, "ms"))
    print("\ncpu_ms summary")
    print(format_summary(fast_cpu_stats, detect_cpu_stats, "ms"))
    print("\nmaxrss_mb summary")
    print(format_summary(fast_rss_stats, detect_rss_stats, "MB"))


if __name__ == "__main__":
    main()
