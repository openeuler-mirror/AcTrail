"""Run alert persistence checks inside the plugin environment scope."""
from __future__ import annotations

from tests.e2e.common.core import TestRunDirectories, TestCase, TestResult
from tests.e2e.common.runner import TestingContextSingleton

from .config import ToolFrequentFailureAlertConfig
from .environment import ToolFrequentFailureAlertEnvironment
from .task import ToolFrequentFailureAlertTask

class ToolFrequentFailureAlertCase(TestCase):
    case_id = 'tool_frequent_failure_alert'
    description = 'Verify the wasm-legacy frequent-failure alert plugin: installed package, window threshold, cooldown, dedup and an optional real-agent round'


    def __init__(self, directories: TestRunDirectories) -> None:
        config = ToolFrequentFailureAlertConfig.from_environment(directories)
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        context = TestingContextSingleton.current()
        with context.case('tool_frequent_failure_alert') as case:
            with ToolFrequentFailureAlertEnvironment(self._config, context.output) as environment:
                with case.step("Prepare the test environment"):
                    environment.prepare()
                ToolFrequentFailureAlertTask(environment).run(case)
        return case.result

