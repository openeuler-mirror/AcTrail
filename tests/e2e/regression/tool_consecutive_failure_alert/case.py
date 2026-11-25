"""Run alert persistence checks inside the plugin environment scope."""
from __future__ import annotations

from tests.e2e.common.core import TestRunDirectories, TestCase, TestResult
from tests.e2e.common.runner import TestingContextSingleton

from .config import ToolConsecutiveFailureAlertConfig
from .environment import ToolConsecutiveFailureAlertEnvironment
from .task import ToolConsecutiveFailureAlertTask

class ToolConsecutiveFailureAlertCase(TestCase):
    case_id = 'tool_consecutive_failure_alert'
    description = 'Verify the wasm-legacy consecutive-failure alert plugin: installed package, threshold/reset/cooldown shell matrix, and an optional real-agent round'


    def __init__(self, directories: TestRunDirectories) -> None:
        config = ToolConsecutiveFailureAlertConfig.from_environment(directories)
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        context = TestingContextSingleton.current()
        with context.case('tool_consecutive_failure_alert') as case:
            with ToolConsecutiveFailureAlertEnvironment(self._config, context.output) as environment:
                with case.step("Prepare the test environment"):
                    environment.prepare()
                ToolConsecutiveFailureAlertTask(environment).run(case)
        return case.result

