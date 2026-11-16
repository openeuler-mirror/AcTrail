#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.e2e.common.runner import TestDefinition, run_one  # noqa: E402
from tests.e2e.regression.tool_frequent_failure_alert.case import (  # noqa: E402
    ToolFrequentFailureAlertCase,
)
from tests.e2e.regression.tool_frequent_failure_alert.config import (  # noqa: E402
    ToolFrequentFailureAlertConfig,
)


TEST_DEFINITION = TestDefinition(
    name="tool_frequent_failure_alert",
    description=(
        "Verify the wasm-legacy frequent-failure alert plugin: installed "
        "package, window threshold, cooldown, dedup and an optional "
        "real-agent round"
    ),
    build_case=lambda inputs: ToolFrequentFailureAlertCase(
        ToolFrequentFailureAlertConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
