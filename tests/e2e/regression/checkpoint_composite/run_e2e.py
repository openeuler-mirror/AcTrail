#!/usr/bin/env python3
"""Run real Xiaoo and Btrfs checkpoint acceptance through the regression runner."""

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.e2e.common.runner import TestDefinition, run_one
from tests.e2e.regression.checkpoint_composite.case import CheckpointCompositeCase

TEST_DEFINITION = TestDefinition(
    name="checkpoint_composite",
    description="Save and restore real Xiaoo context and Btrfs files together",
    build_case=CheckpointCompositeCase,
)

if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
