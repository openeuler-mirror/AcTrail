#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one  # noqa: E402
from tests.v2.regression.network_policy_xiaoo.case import (  # noqa: E402
    NetworkPolicyXiaooCase,
)
from tests.v2.regression.network_policy_xiaoo.config import (  # noqa: E402
    NetworkPolicyXiaooConfig,
)


TEST_DEFINITION = TestDefinition(
    name="network_policy_xiaoo",
    description=(
        "Toggle an exact-endpoint Web rule around real Xiaoo and verify attribution"
    ),
    build_case=lambda inputs: NetworkPolicyXiaooCase(
        NetworkPolicyXiaooConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
