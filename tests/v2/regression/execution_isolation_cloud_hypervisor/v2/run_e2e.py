#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one

from .case import CloudHypervisorExecutionIsolationCase
from .config import CloudHypervisorExecutionIsolationConfig


TEST_DEFINITION = TestDefinition(
    name="execution_isolation_cloud_hypervisor",
    description=(
        "Optional Cloud Hypervisor VMM coverage using the Kata lifecycle "
        "profile and a real xiaoO sandbox observation path"
    ),
    build_case=lambda inputs: CloudHypervisorExecutionIsolationCase(
        CloudHypervisorExecutionIsolationConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
