#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one
from tests.v2.regression.probe_qodercli_llm.case import (  # noqa: E402
    ProbeQoderCliLLMCase,
)
from tests.v2.regression.probe_qodercli_llm.config import (  # noqa: E402
    ProbeQoderCliLLMConfig,
)


TEST_DEFINITION = TestDefinition(
    name="probe_qodercli_llm",
    description="Run qodercli through actrailctl launch and verify LLM capture",
    build_case=lambda inputs: ProbeQoderCliLLMCase(
        ProbeQoderCliLLMConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
