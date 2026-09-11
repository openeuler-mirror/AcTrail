#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one
from tests.v2.regression.resource_metrics_cgroup.case import (  # noqa: E402
    ResourceMetricsCgroupCase,
)


TEST_DEFINITION = TestDefinition(
    name="resource_metrics_cgroup",
    description=(
        "Validate release-binary exact, timeout, and restart cgroup-v2 "
        "resource accounting"
    ),
    build_case=ResourceMetricsCgroupCase,
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
