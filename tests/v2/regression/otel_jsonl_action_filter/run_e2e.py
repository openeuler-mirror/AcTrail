#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one  # noqa: E402
from tests.v2.regression.otel_jsonl_action_filter.case import (  # noqa: E402
    OtelJsonlActionFilterCase,
)
from tests.v2.regression.otel_jsonl_action_filter.config import (  # noqa: E402
    OtelJsonlActionFilterConfig,
)


TEST_DEFINITION = TestDefinition(
    name="otel_jsonl_action_filter",
    description=(
        "Configure builtin otel-jsonl and verify live action-kind filtering"
    ),
    build_case=lambda inputs: OtelJsonlActionFilterCase(
        OtelJsonlActionFilterConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
