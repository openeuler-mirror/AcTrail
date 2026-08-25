#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one
from tests.v2.regression.execution_isolation_cloud_hypervisor.v2.case import (
    CloudHypervisorExecutionIsolationCase,
)

from .config import FirecrackerExecutionIsolationConfig


TEST_DEFINITION = TestDefinition(
    name="execution_isolation_firecracker",
    description=(
        "Firecracker coverage through the Kata lifecycle, its hybrid-VSOCK "
        "bridge and a real xiaoO sandbox observation path"
    ),
    build_case=lambda inputs: CloudHypervisorExecutionIsolationCase(
        FirecrackerExecutionIsolationConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
