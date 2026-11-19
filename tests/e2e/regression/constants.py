from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class RegressionDefaults:
    show_details: bool = False


DEFAULTS = RegressionDefaults()
