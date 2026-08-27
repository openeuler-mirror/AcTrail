from __future__ import annotations

from tests.v2.common.core import TestOutput
from tests.v2.regression.otel_http.environment import OtelHttpEnvironment

from .config import TrajectoryTestConfig


class TrajectoryTestEnvironment(OtelHttpEnvironment):
    def __init__(self, config: TrajectoryTestConfig, output: TestOutput):
        super().__init__(config, output)
