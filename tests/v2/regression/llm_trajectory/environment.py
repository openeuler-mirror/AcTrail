from __future__ import annotations

from tests.v2.common.core import TestOutput
from tests.v2.regression.otel_http.environment import OtelHttpEnvironment

from .config import LlmTrajectoryConfig


class LlmTrajectoryEnvironment(OtelHttpEnvironment):
    def __init__(self, config: LlmTrajectoryConfig, output: TestOutput):
        super().__init__(config, output)

    def configure_trajectory_export(self) -> None:
        self.configure_buffered_export({"llm.request"})
