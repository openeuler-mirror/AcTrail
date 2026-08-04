#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one  # noqa: E402
from tests.v2.regression.probe_codex_mcp.case import (  # noqa: E402
    ProbeCodexMcpCase,
)
from tests.v2.regression.probe_codex_mcp.config import (  # noqa: E402
    ProbeCodexMcpConfig,
)


TEST_DEFINITION = TestDefinition(
    name="probe_codex_mcp",
    description=(
        "Run a real Codex stdio MCP call and verify its fixed semantic graph"
    ),
    build_case=lambda inputs: ProbeCodexMcpCase(
        ProbeCodexMcpConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
