#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.e2e.common.runner import TestDefinition, run_one
from tests.e2e.regression.container_auto.case import (  # noqa: E402
    ContainerAutoCase,
)
from tests.e2e.regression.container_auto.config import (  # noqa: E402
    ContainerAutoConfig,
)


TEST_DEFINITION = TestDefinition(
    name="container_auto",
    description=(
        "Run the ordinary Docker deployment permission matrix and isolation acceptance"
    ),
    build_case=lambda inputs: ContainerAutoCase(
        ContainerAutoConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
