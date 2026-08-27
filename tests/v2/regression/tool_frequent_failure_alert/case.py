from __future__ import annotations

from tests.v2.common.core import TestCase, TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton

from .config import ToolFrequentFailureAlertConfig
from .environment import ToolFrequentFailureAlertEnvironment
from .task import ToolFrequentFailureAlertTask


class ToolFrequentFailureAlertCase(TestCase):
    def __init__(self, config: ToolFrequentFailureAlertConfig):
        self._config = config
        self._environment: ToolFrequentFailureAlertEnvironment | None = None

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        try:
            test_context.report_progress(
                "environment_prepare",
                "starting actraild and loading the alert plugin",
            )
            self._environment = ToolFrequentFailureAlertEnvironment(
                self._config,
                test_context.output,
            )
            self._environment.prepare()
            results["environment"] = TestResult(
                TestStatus.PASSED,
                "actraild active and tool-frequent-alert plugin loaded",
            )

            task = ToolFrequentFailureAlertTask(
                self._environment,
                test_context,
            )
            results.update(task.run())
            return TestResult(
                TestStatus.COMPOSITE,
                "tool frequent-failure alert persistence",
                results,
            )
        except Exception as error:
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
            return TestResult(
                TestStatus.COMPOSITE,
                "tool frequent-failure alert persistence",
                results,
            )

    def cleanup(
        self,
        test_context: TestingContextSingleton,
    ) -> TestResult | None:
        del test_context
        if self._environment is None:
            return None
        return self._environment.cleanup()
