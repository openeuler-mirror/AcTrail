from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class RegressionDefaults:
    profile: Path = Path("local/kata/v2-test-profile.json")
    show_details: bool = False


DEFAULTS = RegressionDefaults()
