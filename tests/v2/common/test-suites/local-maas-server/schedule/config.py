from __future__ import annotations

import math
from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class ScheduleConfig:
    ttft_seconds: float
    tpot_seconds: float

    def __post_init__(self) -> None:
        for value, name in (
            (self.ttft_seconds, "ttft_seconds"),
            (self.tpot_seconds, "tpot_seconds"),
        ):
            if not math.isfinite(value):
                raise ValueError(f"{name} must be finite")
        if self.ttft_seconds < 0:
            raise ValueError("ttft_seconds must be non-negative")
        if self.tpot_seconds < 0:
            raise ValueError("tpot_seconds must be non-negative")
