#!/usr/bin/env python3
from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one
from tests.v2.regression.resource_storage_retention.case import ResourceStorageRetentionCase

TEST_DEFINITION = TestDefinition(
    name="resource_storage_retention",
    description="Validate resource SQLite backlog cleanup, fresh rows and profile/boot reclamation",
    build_case=ResourceStorageRetentionCase,
)

if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
