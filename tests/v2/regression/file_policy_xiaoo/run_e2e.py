#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one  # noqa: E402
from tests.v2.regression.file_policy_xiaoo.case import FilePolicyXiaooCase  # noqa: E402


TEST_DEFINITION = TestDefinition(
    name="file_policy_xiaoo",
    description="Run real Xiaoo file writes under default complete LSM and explicit fanotify rules",
    build_case=FilePolicyXiaooCase,
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
