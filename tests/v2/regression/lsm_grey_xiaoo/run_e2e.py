#!/usr/bin/env python3
"""Run the real Xiaoo LSM grey authorization scenario."""
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))
from tests.v2.common.runner import TestDefinition, run_one
from tests.v2.regression.lsm_grey_xiaoo.case import LsmGreyXiaooCase

TEST_DEFINITION = TestDefinition(
    name="lsm_grey_xiaoo",
    description="Real Xiaoo LSM authorization through a WASM plugin and external HTTP security service",
    build_case=LsmGreyXiaooCase,
)
if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
