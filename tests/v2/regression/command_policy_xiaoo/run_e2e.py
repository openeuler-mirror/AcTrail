#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one  # noqa: E402
from tests.v2.regression.command_policy_xiaoo.case import (  # noqa: E402
    CommandPolicyXiaooCase,
)
from tests.v2.regression.command_policy_xiaoo.config import (  # noqa: E402
    CommandPolicyXiaooConfig,
)


TEST_DEFINITION = TestDefinition(
    name="command_policy_xiaoo",
    description=(
        "Use Web configuration to deny real Xiaoo Bash execution and verify governance"
    ),
    build_case=lambda inputs: CommandPolicyXiaooCase(
        CommandPolicyXiaooConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
