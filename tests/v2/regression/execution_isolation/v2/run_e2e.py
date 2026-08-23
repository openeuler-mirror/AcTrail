#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one

from .case import ExecutionIsolationCase
from .config import ExecutionIsolationConfig


TEST_DEFINITION = TestDefinition(
    name="execution_isolation",
    description=(
        "Run real xiaoO through actrail-sb, Cloud Hypervisor VSOCK gateway "
        "and daemon sandbox plugin"
    ),
    build_case=lambda inputs: ExecutionIsolationCase(
        ExecutionIsolationConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
