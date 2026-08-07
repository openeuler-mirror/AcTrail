from __future__ import annotations

from tests.v2.common.agent_selection import AgentSelector
from tests.v2.common.core import TestCase, TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton

from .config import OtelJsonlActionFilterConfig
from .environment import OtelJsonlActionFilterEnvironment
from .task import OtelJsonlActionFilterTask


class OtelJsonlActionFilterCase(TestCase):
    def __init__(self, config: OtelJsonlActionFilterConfig):
        self._config = config
        self._environment: OtelJsonlActionFilterEnvironment | None = None

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        try:
            test_context.report_progress(
                "agent_selection",
                "selecting an available agent",
            )
            agent = AgentSelector(self._config.repo).select(test_context)
            if agent is None:
                return TestResult(
                    TestStatus.SKIPPED,
                    "no usable agent binary in "
                    "xiaoo/pi/opencode/claude/codex",
                )
            results["agent_detection"] = TestResult(
                TestStatus.PASSED,
                f"selected {agent.kind}: {agent.binary}",
            )

            test_context.report_progress(
                "environment_prepare",
                "starting actraild, actrailweb, and otel-jsonl",
            )
            self._environment = OtelJsonlActionFilterEnvironment(
                self._config,
                test_context.output,
            )
            self._environment.prepare()
            results["environment"] = TestResult(
                TestStatus.PASSED,
                "actraild, actrailweb, and builtin otel-jsonl are active",
            )

            task = OtelJsonlActionFilterTask(
                self._environment,
                agent,
                test_context,
            )
            results.update(task.run())
            return TestResult(
                TestStatus.COMPOSITE,
                "otel-jsonl action filtering",
                results,
            )
        except Exception as error:
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
            return TestResult(
                TestStatus.COMPOSITE,
                "otel-jsonl action filtering",
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
