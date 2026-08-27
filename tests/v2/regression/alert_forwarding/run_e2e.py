#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.runner import TestDefinition, run_one  # noqa: E402
from tests.v2.regression.alert_forwarding.case import AlertForwardingCase  # noqa: E402
from tests.v2.regression.alert_forwarding.config import (  # noqa: E402
    AlertForwardingRegressionConfig,
)


TEST_DEFINITION = TestDefinition(
    name="alert_forwarding",
    description=(
        "Verify daemon auto-launch, subscriber fanout, and a real low-threshold "
        "stored-alert forwarding path"
    ),
    build_case=lambda inputs: AlertForwardingCase(
        AlertForwardingRegressionConfig.from_environment(inputs)
    ),
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
