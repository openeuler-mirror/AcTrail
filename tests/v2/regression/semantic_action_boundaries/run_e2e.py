#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one  # noqa: E402
from tests.v2.regression.semantic_action_boundaries.case import (  # noqa: E402
    SemanticActionBoundariesCase,
)
from tests.v2.regression.semantic_action_boundaries.config import (  # noqa: E402
    SemanticActionBoundariesConfig,
)


TEST_DEFINITION = TestDefinition(
    name="semantic_action_boundaries",
    description=(
        "Run a real agent and verify semantic action terminal boundaries"
    ),
    build_case=lambda inputs: SemanticActionBoundariesCase(
        SemanticActionBoundariesConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
