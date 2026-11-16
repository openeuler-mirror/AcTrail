#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.e2e.common.runner import run_one
from tests.e2e.regression.execution_isolation_cloud_hypervisor.v2.run_e2e import (
    TEST_DEFINITION,
)

__all__ = ["TEST_DEFINITION"]


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
