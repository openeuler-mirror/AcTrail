#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one
from tests.v2.regression.probe_claude_mcp.case import (  # noqa: E402
    ProbeClaudeMcpCase,
)
from tests.v2.regression.probe_claude_mcp.config import (  # noqa: E402
    ProbeClaudeMcpConfig,
)


TEST_DEFINITION = TestDefinition(
    name="probe_claude_mcp",
    description=(
        "Run a real Claude stdio MCP call and verify its fixed semantic graph"
    ),
    build_case=lambda inputs: ProbeClaudeMcpCase(
        ProbeClaudeMcpConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
