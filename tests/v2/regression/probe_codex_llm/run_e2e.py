#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one  # noqa: E402
from tests.v2.regression.probe_codex_llm.case import ProbeCodexLLMCase  # noqa: E402
from tests.v2.regression.probe_codex_llm.config import (  # noqa: E402
    ProbeCodexLLMConfig,
)


TEST_DEFINITION = TestDefinition(
    name="probe_codex_llm",
    description="Run Codex through nested actrailctl launch and verify LLM capture",
    build_case=lambda repo, bin_dir: ProbeCodexLLMCase(
        ProbeCodexLLMConfig.from_environment(repo, bin_dir)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
