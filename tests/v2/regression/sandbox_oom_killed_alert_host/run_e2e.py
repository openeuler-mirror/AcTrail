#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one
from tests.v2.regression.sandbox_oom_killed_alert_host.case import (  # noqa: E402
    SandboxOomKilledAlertHostCase,
)
from tests.v2.regression.sandbox_oom_killed_alert_host.config import (  # noqa: E402
    SandboxOomKilledAlertHostConfig,
)


TEST_DEFINITION = TestDefinition(
    name="sandbox_oom_killed_alert_host",
    description=(
        "Validate one controlled host cgroup OOM as a focused "
        "sandbox.resource.oom_killed public alert"
    ),
    build_case=lambda inputs: SandboxOomKilledAlertHostCase(
        SandboxOomKilledAlertHostConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
