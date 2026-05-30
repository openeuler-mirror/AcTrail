#!/usr/bin/env python3
"""Run AcTrail platform and agent TLS readiness checks."""

from __future__ import annotations

import sys
from pathlib import Path


DOCS_DIR = Path(__file__).resolve().parents[1]
if str(DOCS_DIR) not in sys.path:
    sys.path.insert(0, str(DOCS_DIR))

from preflight.runner import main


if __name__ == "__main__":
    raise SystemExit(main())
