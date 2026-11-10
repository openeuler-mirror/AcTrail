#!/usr/bin/env python3
from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one
from tests.v2.regression.checkpoint_btrfs.case import CheckpointBtrfsCase

TEST_DEFINITION = TestDefinition(
    name="checkpoint_btrfs",
    description="Save and restore a launched Agent's files with the Btrfs checkpoint backend",
    build_case=CheckpointBtrfsCase,
)

if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
