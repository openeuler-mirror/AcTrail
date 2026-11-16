#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.e2e.common.runner import TestDefinition, run_one  # noqa: E402
from tests.e2e.regression.lsm_access_control_xiaoo.case import LsmAccessControlXiaooCase  # noqa: E402


TEST_DEFINITION = TestDefinition(
    name="lsm_access_control_xiaoo",
    description="Real Xiaoo file, exec, and network enforcement through BPF LSM with bpf-copy observation",
    build_case=LsmAccessControlXiaooCase,
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
