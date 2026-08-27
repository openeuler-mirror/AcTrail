#!/usr/bin/env python3
"""Run AcTrail read-only platform readiness checks."""

from __future__ import annotations

import sys
from pathlib import Path


SCRIPTS_DIR = Path(__file__).resolve().parents[1]
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

from preflight.runner import main


if __name__ == "__main__":
    raise SystemExit(main())
