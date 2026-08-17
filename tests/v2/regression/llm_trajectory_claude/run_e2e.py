#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one
from tests.v2.regression.llm_trajectory_claude.case import (  # noqa: E402
    ClaudeTrajectoryCase,
)
from tests.v2.regression.llm_trajectory_claude.config import (  # noqa: E402
    ClaudeTrajectoryConfig,
)


TEST_DEFINITION = TestDefinition(
    name="llm_trajectory_claude",
    description="Verify real Claude subagent LLM trajectory identification",
    build_case=lambda inputs: ClaudeTrajectoryCase(
        ClaudeTrajectoryConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
