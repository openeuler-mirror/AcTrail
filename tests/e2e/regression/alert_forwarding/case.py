from __future__ import annotations

from tests.e2e.common.core import TestRunDirectories, TestCase, TestResult, TestStatus
from tests.e2e.common.runner import TestingContextSingleton

from .config import AlertForwardingRegressionConfig
from .environment import AlertForwardingEnvironment
from .scenario import AlertForwardingScenario


class AlertForwardingCase(TestCase):
    case_id = 'alert_forwarding'
    description = 'Verify daemon auto-launch, subscriber fanout, and a real low-threshold stored-alert forwarding path'


    def __init__(self, directories: TestRunDirectories) -> None:
        config = AlertForwardingRegressionConfig.from_environment(directories)
        self._config = config
        self._environment: AlertForwardingEnvironment | None = None

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        try:
            test_context.report_progress(
                "Start the observation services",
                "starting isolated daemon, auto-launched proxy, and real alert plugin",
            )
            self._environment = AlertForwardingEnvironment(
                self._config,
                test_context.output,
            )
            self._environment.prepare()
            results.update(
                AlertForwardingScenario(self._environment, test_context).run()
            )
        except Exception as error:
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
        return TestResult(
            TestStatus.COMPOSITE,
            "real alert forwarding path",
            results,
        )

    def cleanup(self, test_context: TestingContextSingleton) -> TestResult | None:
        del test_context
        if self._environment is None:
            return None
        return self._environment.cleanup()
