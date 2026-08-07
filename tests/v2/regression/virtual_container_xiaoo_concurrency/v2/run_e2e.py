#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one
from tests.v2.regression.virtual_container_xiaoo_concurrency.v2 import (  # noqa: E402
    case as case_module,
    config as config_module,
)

VirtualContainerXiaooConcurrencyCase = (
    case_module.VirtualContainerXiaooConcurrencyCase
)
VirtualContainerXiaooConcurrencyConfig = (
    config_module.VirtualContainerXiaooConcurrencyConfig
)


TEST_DEFINITION = TestDefinition(
    name="virtual_container_xiaoo_concurrency",
    description=(
        "Run two overlapping xiaoO workloads in two independently managed Kata VMs"
    ),
    build_case=lambda inputs: VirtualContainerXiaooConcurrencyCase(
        VirtualContainerXiaooConcurrencyConfig.from_environment(inputs)
    ),
    skip_if_skipped=("virtual_container",),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
