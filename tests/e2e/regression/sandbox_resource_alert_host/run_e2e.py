#!/usr/bin/env python3
"""Run the active regression scenario."""
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.e2e.common.runner import run_one
from tests.e2e.regression.sandbox_resource_alert_host.v2.case import SandboxResourceAlertHostCase

if __name__ == "__main__":
    raise SystemExit(run_one(SandboxResourceAlertHostCase, REPO))
