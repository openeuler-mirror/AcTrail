#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one  # noqa: E402
from tests.v2.regression.tool_consecutive_failure_alert.case import (  # noqa: E402
    ToolConsecutiveFailureAlertCase,
)
from tests.v2.regression.tool_consecutive_failure_alert.config import (  # noqa: E402
    ToolConsecutiveFailureAlertConfig,
)


TEST_DEFINITION = TestDefinition(
    name="tool_consecutive_failure_alert",
    description=(
        "Verify the wasm-legacy consecutive-failure alert plugin: installed "
        "package, threshold/reset/cooldown shell matrix, and an optional "
        "real-agent round"
    ),
    build_case=lambda inputs: ToolConsecutiveFailureAlertCase(
        ToolConsecutiveFailureAlertConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
