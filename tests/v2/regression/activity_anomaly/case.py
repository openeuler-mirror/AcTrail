from __future__ import annotations

from tests.v2.common.core import TestCase, TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton

from .config import ActivityAnomalyConfig
from .environment import ActivityAnomalyEnvironment
from .task import ActivityAnomalyTask


class ActivityAnomalyCase(TestCase):
    def __init__(self, config: ActivityAnomalyConfig):
        self._config = config
        self._environment: ActivityAnomalyEnvironment | None = None

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        try:
            test_context.report_progress(
                "environment_prepare",
                "starting isolated services and loading installed activity plugin",
            )
            self._environment = ActivityAnomalyEnvironment(
                self._config,
                test_context.output,
            )
            self._environment.prepare()
            results["environment"] = TestResult(
                TestStatus.PASSED,
                "isolated daemon active and installed activity plugin loaded",
            )
            results.update(ActivityAnomalyTask(self._environment, test_context).run())
        except Exception as error:
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
        return TestResult(
            TestStatus.COMPOSITE,
            "real Xiaoo activity-anomaly alert flow",
            results,
        )

    def cleanup(self, test_context: TestingContextSingleton) -> TestResult | None:
        del test_context
        if self._environment is None:
            return None
        return self._environment.cleanup()
