#!/usr/bin/env python3
"""Compare operation costs without and with AcTrail observation."""

from __future__ import annotations

import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT))

from scripts.bench.detail.benchmark import main  # noqa: E402


if __name__ == "__main__":
    raise SystemExit(main())
