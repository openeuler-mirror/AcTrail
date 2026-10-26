#!/usr/bin/env python3
"""Run the collection configuration × workload benchmark."""

import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT))
sys.path.insert(0, str(REPO_ROOT / "tests/v2/common/test_suites/local_maas_server"))

from scripts.bench.payload.benchmark import main  # noqa: E402

if __name__ == "__main__":
    raise SystemExit(main())
