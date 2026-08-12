#!/usr/bin/env python3
"""Benchmark replay of a recorded scenario: bare agent vs actrail-managed agent."""

from __future__ import annotations

import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
SERVER_DIR = REPO_ROOT / "tests/v2/common/test_suites/local_maas_server"
sys.path.insert(0, str(REPO_ROOT))
sys.path.insert(0, str(SERVER_DIR))

from scripts.bench.overall.benchmark import main  # noqa: E402


if __name__ == "__main__":
    raise SystemExit(main())
