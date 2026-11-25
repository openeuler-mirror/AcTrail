"""Run otel-jsonl action filtering with scoped resources and step results."""
from __future__ import annotations

from tests.e2e.common.agents import AgentFactory
from tests.e2e.common.core import TestRunDirectories, TestCase, TestResult
from tests.e2e.common.runner import TestingContextSingleton
from tests.e2e.common.testing_env import AgentBinaryDiscovery
from .config import OtelJsonlActionFilterConfig
from .environment import OtelJsonlActionFilterEnvironment
from .task import OtelJsonlActionFilterTask


class OtelJsonlActionFilterCase(TestCase):
    case_id = 'otel_jsonl_action_filter'
    description = 'Select builtin OTEL exporters and verify live action-kind filtering'


    def __init__(self, directories: TestRunDirectories) -> None:
        config = OtelJsonlActionFilterConfig.from_environment(directories)
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        context = TestingContextSingleton.current()
        with context.case("otel-jsonl action filtering") as case:
            with case.step("Select an Agent"):
                agent = AgentFactory.auto(self._config.repo, context)
                if agent is None:
                    case.skip("no usable agent binary in xiaoo/pi/opencode/claude/codex")
                binary = agent.binary
                if agent.kind == "codex":
                    binary = AgentBinaryDiscovery(self._config.repo).codex_native_binary_for(binary)
                    if binary is None:
                        case.skip("known Codex native executable is unavailable")
            with OtelJsonlActionFilterEnvironment(self._config, context.output, binary) as environment:
                with case.step("Prepare the test environment"):
                    environment.prepare()
                OtelJsonlActionFilterTask(environment, agent).run(case)
        return case.result
