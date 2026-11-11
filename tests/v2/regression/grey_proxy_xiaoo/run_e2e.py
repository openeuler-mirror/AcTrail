#!/usr/bin/env python3
"""Run context-enhanced grey authorization through a real Xiaoo process."""
import sys
from pathlib import Path
REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))
from tests.v2.common.runner import TestDefinition, run_one
from tests.v2.regression.grey_proxy_xiaoo.case import GreyProxyXiaooCase
TEST_DEFINITION = TestDefinition(name="grey_proxy_xiaoo", description="Managed grey proxy with real process history", build_case=GreyProxyXiaooCase)
if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
