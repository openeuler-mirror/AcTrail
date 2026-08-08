#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one
from tests.v2.regression.container_auto.v2 import (  # noqa: E402
    ContainerAutoCase,
    ContainerAutoConfig,
)


TEST_DEFINITION = TestDefinition(
    name="container_auto",
    description="Verify Docker host-eBPF and seccomp-notify auto-selection",
    build_case=lambda inputs: ContainerAutoCase(
        ContainerAutoConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
