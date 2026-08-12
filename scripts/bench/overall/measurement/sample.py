"""Measurement result for one benchmark case."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class Sample:
    wall_seconds: float
    cpu_seconds: float
    peak_rss_kb: int
    extra_cpu_seconds: float = 0.0
    extra_peak_rss_kb: int = 0

    @property
    def wall_ms(self) -> float:
        return self.wall_seconds * 1000

    @property
    def cpu_ms(self) -> float:
        return self.cpu_seconds * 1000

    @property
    def peak_rss_mb(self) -> float:
        return self.peak_rss_kb / 1024

    @property
    def cpu_percent(self) -> float:
        if self.wall_seconds <= 0:
            return float("nan")
        return self.cpu_seconds / self.wall_seconds * 100
