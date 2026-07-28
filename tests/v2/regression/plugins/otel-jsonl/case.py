from __future__ import annotations

from tests.v2.common.agent_selection import AgentSelector
from tests.v2.common.test_case import TestCase, TestResult, TestStatus
from tests.v2.common.testing_context import TestingContextSingleton

from .config import OtelJsonlConfig
from .environment import OtelJsonlEnvironment
from .task import OtelJsonlTask


class OtelJsonlCase(TestCase):
    def __init__(self, config: OtelJsonlConfig):
        self._config = config
        self._environment: OtelJsonlEnvironment | None = None

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        try:
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

            self._environment = OtelJsonlEnvironment(
                self._config,
                test_context.output,
            )
            self._environment.prepare()
            results["environment"] = TestResult(
                TestStatus.PASSED,
                "actraild, actrailweb, and builtin otel-jsonl are active",
            )

            task = OtelJsonlTask(self._environment, agent)
            results.update(task.run())
            return TestResult(
                TestStatus.COMPOSITE,
                "otel-jsonl Web action filter",
                results,
            )
        except Exception as error:
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
            return TestResult(
                TestStatus.COMPOSITE,
                "otel-jsonl Web action filter",
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
