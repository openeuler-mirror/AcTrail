#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one
from tests.v2.regression.virtual_container.v2.case import (  # noqa: E402
    VirtualContainerCase,
)
from tests.v2.regression.virtual_container.v2.config import (  # noqa: E402
    VirtualContainerConfig,
)


TEST_DEFINITION = TestDefinition(
    name="virtual_container",
    description=(
        "Validate deployment contracts and run the reusable Kata V2 "
        "interface/data matrix"
    ),
    build_case=lambda inputs: VirtualContainerCase(
        VirtualContainerConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
