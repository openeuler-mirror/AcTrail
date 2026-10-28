#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one  # noqa: E402
from tests.v2.regression.context_compact.case import ContextCompactCase  # noqa: E402
from tests.v2.regression.context_compact.config import (  # noqa: E402
    ContextCompactConfig,
)


TEST_DEFINITION = TestDefinition(
    name="context_compact",
    description="Auto-detect a real agent and run its three-turn compact workflow",
    build_case=lambda inputs: ContextCompactCase(
        ContextCompactConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
