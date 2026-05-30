#!/usr/bin/env python3
"""Run AcTrail regression suites."""

from __future__ import annotations

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent
UTILS = ROOT / "00-utils"
if str(UTILS) not in sys.path:
    sys.path.insert(0, str(UTILS))

from runner import main  # noqa: E402


if __name__ == "__main__":
    raise SystemExit(main(ROOT))
