from __future__ import annotations

from tests.v2.common.agent_selection import AgentSelector
from tests.v2.common.core import TestCase, TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton

from .config import SemanticActionBoundariesConfig
from .environment import SemanticActionBoundariesEnvironment
from .task import SemanticActionBoundariesTask


class SemanticActionBoundariesCase(TestCase):
    def __init__(self, config: SemanticActionBoundariesConfig):
        self._config = config
        self._environment: SemanticActionBoundariesEnvironment | None = None

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
                "starting actraild and semantic observation",
            )
            self._environment = SemanticActionBoundariesEnvironment(
                self._config,
                test_context.output,
            )
            self._environment.prepare()
            results["environment"] = TestResult(
                TestStatus.PASSED,
                "actraild and semantic action observation are active",
            )

            task = SemanticActionBoundariesTask(
                self._environment,
                agent,
                test_context,
            )
            results.update(task.run())
            return TestResult(
                TestStatus.COMPOSITE,
                "semantic action export boundaries",
                results,
            )
        except Exception as error:
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
            return TestResult(
                TestStatus.COMPOSITE,
                "semantic action export boundaries",
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
