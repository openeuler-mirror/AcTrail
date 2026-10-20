#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one
from tests.v2.regression.probe_dcode_llm.case import ProbeDcodeLLMCase  # noqa: E402
from tests.v2.regression.probe_dcode_llm.config import ProbeDcodeLLMConfig  # noqa: E402


TEST_DEFINITION = TestDefinition(
    name="probe_dcode_llm",
    description="Run dcode through actrailctl launch and verify LLM capture",
    build_case=lambda inputs: ProbeDcodeLLMCase(
        ProbeDcodeLLMConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
