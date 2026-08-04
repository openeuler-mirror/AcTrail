#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one  # noqa: E402
from tests.v2.regression.container_agent_xiaoo.v2.case import (  # noqa: E402
    ContainerAgentXiaooCase,
)
from tests.v2.regression.container_agent_xiaoo.v2.config import (  # noqa: E402
    ContainerAgentXiaooConfig,
)


TEST_DEFINITION = TestDefinition(
    name="container_agent_xiaoo",
    description=(
        "Run two real xiaoO agents in Docker and verify independent traces"
    ),
    build_case=lambda inputs: ContainerAgentXiaooCase(
        ContainerAgentXiaooConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
