#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one

from .case import SandboxResourceAlertHostCase
from .config import SandboxResourceAlertHostConfig


TEST_DEFINITION = TestDefinition(
    name="sandbox_resource_alert_host",
    description=(
        "Run real xiaoO through host actrail-sb, native VSOCK gateway, daemon "
        "sandbox alert storage and alert-proxy"
    ),
    build_case=lambda inputs: SandboxResourceAlertHostCase(
        SandboxResourceAlertHostConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
