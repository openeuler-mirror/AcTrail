#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one
from tests.v2.regression.activity_anomaly.case import ActivityAnomalyCase
from tests.v2.regression.activity_anomaly.config import ActivityAnomalyConfig


TEST_DEFINITION = TestDefinition(
    name="plugin_activity_anomaly",
    description=(
        "Run one real Xiaoo loop and verify request growth, response growth, "
        "and command-duration alerts from the installed activity-anomaly plugin"
    ),
    build_case=lambda inputs: ActivityAnomalyCase(
        ActivityAnomalyConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
