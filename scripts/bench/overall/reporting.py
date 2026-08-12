"""Summary and report generation for the overall benchmark."""

from __future__ import annotations

from dataclasses import dataclass, field
import statistics
from typing import Any

from .measurement import Sample


def summarize(values: list[float]) -> dict[str, float]:
    ordered = sorted(values)
    return {
        "mean": statistics.mean(values),
        "median": statistics.median(values),
        "p95": ordered[int(len(ordered) * 0.95) - 1],
        "min": ordered[0],
        "max": ordered[-1],
    }


@dataclass(slots=True)
class Report:
    """Incrementally filled benchmark inputs; serialization in to_dict."""

    commit_id: str = ""
    commit_title: str = ""
    scenario: str = ""
    agent: str = ""
    rounds: int = 0
    max_turns: int = 0
    bare_samples: list[Sample] = field(default_factory=list)
    actrail_samples: list[Sample] = field(default_factory=list)
    actrail_baselines_ms: list[float] = field(default_factory=list)
    storage_footprint_bytes: int = 0

    def to_dict(self) -> dict[str, Any]:
        return {
            "commit": {
                "id": self.commit_id,
                "title": self.commit_title,
            },
            "scenario": self.scenario,
            "agent": self.agent,
            "rounds": self.rounds,
            "kept_per_case": self.rounds,
            "warmup_discarded": True,
            "max_turns": self.max_turns,
            "per_round": {
                "bare": [
                    {
                        "wall_ms": sample.wall_ms,
                        "cpu_ms": sample.cpu_ms,
                        "peak_rss_mb": sample.peak_rss_mb,
                        "daemon_cpu_ms": sample.extra_cpu_seconds * 1000,
                    }
                    for sample in self.bare_samples
                ],
                "actrail": [
                    {
                        "wall_ms": sample.wall_ms,
                        "cpu_ms": sample.cpu_ms,
                        "peak_rss_mb": sample.peak_rss_mb,
                        "daemon_cpu_ms": sample.extra_cpu_seconds * 1000,
                        "daemon_peak_rss_mb": (
                            sample.extra_peak_rss_kb / 1024
                        ),
                    }
                    for sample in self.actrail_samples
                ],
            },
            "bare": {
                "wall_ms": summarize(
                    [sample.wall_ms for sample in self.bare_samples]
                ),
                "cpu_ms": summarize(
                    [sample.cpu_ms for sample in self.bare_samples]
                ),
                "peak_rss_mb": summarize(
                    [sample.peak_rss_mb for sample in self.bare_samples]
                ),
            },
            "actrail": {
                "wall_ms": summarize(
                    [sample.wall_ms for sample in self.actrail_samples]
                ),
                "cpu_ms": summarize(
                    [sample.cpu_ms for sample in self.actrail_samples]
                ),
                "peak_rss_mb": summarize(
                    [sample.peak_rss_mb for sample in self.actrail_samples]
                ),
                "actraild_cpu_ms": summarize(
                    [
                        sample.extra_cpu_seconds * 1000
                        for sample in self.actrail_samples
                    ]
                ),
                "actraild_baseline_cpu_ms": summarize(
                    self.actrail_baselines_ms
                ),
                "actraild_peak_rss_mb": summarize(
                    [
                        sample.extra_peak_rss_kb / 1024
                        for sample in self.actrail_samples
                    ]
                ),
                "storage_footprint_mb": (
                    self.storage_footprint_bytes / (1024 * 1024)
                ),
            },
        }


def print_comparison(bare: list[Sample], actrail: list[Sample]) -> None:
    def ratio(value: float, base: float) -> float:
        return value / base if base else float("nan")

    bare_wall = [sample.wall_ms for sample in bare]
    actrail_wall = [sample.wall_ms for sample in actrail]
    bare_cpu = [sample.cpu_ms for sample in bare]
    actrail_cpu = [sample.cpu_ms for sample in actrail]
    bare_rss = [sample.peak_rss_mb for sample in bare]
    actrail_rss = [sample.peak_rss_mb for sample in actrail]
    extra_cpu = [sample.extra_cpu_seconds * 1000 for sample in actrail]
    extra_rss = [sample.extra_peak_rss_kb / 1024 for sample in actrail]

    print(f"{'metric':>14} {'stat':>8} {'bare':>12} {'actrail':>12} {'ratio':>7}")
    for label, bare_values, actrail_values, digits in (
        ("wall(ms)", bare_wall, actrail_wall, 0),
        ("cpu(ms)", bare_cpu, actrail_cpu, 0),
        ("rss(MB)", bare_rss, actrail_rss, 1),
    ):
        bare_stats = summarize(bare_values)
        actrail_stats = summarize(actrail_values)
        for stat_name, key in (("mean", "mean"), ("median", "median")):
            bare_value = bare_stats[key]
            actrail_value = actrail_stats[key]
            row_ratio = ratio(actrail_value, bare_value)
            print(
                f"{label:>14} {stat_name:>8} "
                f"{bare_value:>12.{digits}f} "
                f"{actrail_value:>12.{digits}f} "
                f"{row_ratio:>6.2f}x"
            )
    if any(extra_cpu):
        print(
            "actraild: cpu "
            f"{statistics.mean(extra_cpu):.0f}ms rss "
            f"{statistics.mean(extra_rss):.1f}MB"
        )
