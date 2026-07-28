#!/usr/bin/env python3

from __future__ import annotations

import sys
from importlib import import_module
from pathlib import Path

REPO = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one  # noqa: E402

OtelJsonlCase = import_module(
    "tests.v2.regression.plugins.otel-jsonl.case"
).OtelJsonlCase
OtelJsonlConfig = import_module(
    "tests.v2.regression.plugins.otel-jsonl.config"
).OtelJsonlConfig


TEST_DEFINITION = TestDefinition(
    name="plugin_otel_jsonl",
    description=(
        "Configure builtin otel-jsonl through Web APIs and verify live action filtering"
    ),
    build_case=lambda inputs: OtelJsonlCase(
        OtelJsonlConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
