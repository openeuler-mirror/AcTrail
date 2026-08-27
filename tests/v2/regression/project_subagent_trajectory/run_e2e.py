#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one
from tests.v2.regression.project_subagent_trajectory.case import (  # noqa: E402
    ProjectSubagentTrajectoryCase,
)
from tests.v2.regression.project_subagent_trajectory.config import (  # noqa: E402
    ProjectSubagentTrajectoryConfig,
)


TEST_DEFINITION = TestDefinition(
    name="project_subagent_trajectory",
    description="Verify configurable project-subagent LLM projection",
    build_case=lambda inputs: ProjectSubagentTrajectoryCase(
        ProjectSubagentTrajectoryConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
