#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one
from tests.v2.regression.otel_http.case import OtelHttpCase  # noqa: E402
from tests.v2.regression.otel_http.config import OtelHttpConfig  # noqa: E402


TEST_DEFINITION = TestDefinition(
    name="otel_http",
    description=(
        "Verify builtin OTEL/HTTP terminal, metadata-only, and shutdown boundaries"
    ),
    build_case=lambda inputs: OtelHttpCase(OtelHttpConfig.from_environment(inputs)),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
