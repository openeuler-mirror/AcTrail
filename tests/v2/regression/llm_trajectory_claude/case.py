from __future__ import annotations

from tests.v2.common.core import TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton
from tests.v2.common.testing_env import AgentBinaryDiscovery
from tests.v2.regression.llm_trajectory.assertion import (
    ProductAssertionFailure,
    ScenarioPreconditionFailure,
)
from tests.v2.regression.llm_trajectory.case import LlmTrajectoryCase
from tests.v2.regression.llm_trajectory.environment import LlmTrajectoryEnvironment
from tests.v2.regression.llm_trajectory.scenario import FixtureRepository

from .assertion import ClaudeTrajectoryAssertion
from .config import ClaudeTrajectoryConfig
from .scenario import ClaudeSubagentScenario


class ClaudeTrajectoryCase(LlmTrajectoryCase):
    def __init__(self, config: ClaudeTrajectoryConfig):
        super().__init__(config)
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        try:
            discovery = AgentBinaryDiscovery(self._config.repo)
            claude = discovery.resolve("CLAUDE_E2E_BINARY", "claude")
            if claude is None:
                return TestResult(
                    TestStatus.SKIPPED,
                    "Claude executable not found; set CLAUDE_E2E_BINARY",
                )
            claude_environment = discovery.environment(claude)
            test_context.report_progress(
                "agent_availability",
                "checking Claude availability before starting capture",
            )
            if not test_context.check_agent_availability(
                "claude",
                claude,
                claude_environment,
            ):
                return TestResult(
                    TestStatus.SKIPPED,
                    "Claude external availability check failed",
                )

            test_context.report_progress(
                "environment_prepare",
                "starting actraild, actrailweb, OTel HTTP, and local receiver",
            )
            self._environment = LlmTrajectoryEnvironment(
                self._config,
                test_context.output,
            )
            self._environment.prepare()
            self._environment.configure_trajectory_export()
            results["infrastructure"] = TestResult(
                TestStatus.PASSED,
                "refreshed daemon, Web API, and llm.request exporter are ready",
            )

            fixture = FixtureRepository.create(
                self._environment.runtime,
                self._config.work_dir,
            )
            scenario = ClaudeSubagentScenario(
                self._environment.runtime,
                claude,
                claude_environment,
                fixture,
                self._config.model,
                self._config.launch_timeout_seconds,
            )
            test_context.report_progress(
                "agent_launch",
                "running Claude with a mandatory general-purpose Agent git task",
            )
            trace_id, launch = scenario.run()
            if scenario.answer_marker not in launch.stdout:
                raise ScenarioPreconditionFailure(
                    f"Claude final output has no answer marker {scenario.answer_marker}"
                )
            if fixture.commit_id not in launch.stdout:
                raise ScenarioPreconditionFailure(
                    "Claude final output did not report the fixture commit"
                )
            self._wait_for_terminal_trace(trace_id)
            results["agent-run"] = TestResult(
                TestStatus.PASSED,
                f"real Claude trace-{trace_id} returned the fixture commit",
            )

            assertion = ClaudeTrajectoryAssertion(
                self._environment,
                trace_id,
                scenario.task_marker,
                fixture.commit_id,
                self._config.request_content_max_bytes,
            )
            test_context.report_progress(
                "scenario_preconditions",
                "proving Agent/Task, delegated Bash, and both continuations",
            )
            evidence = self._wait_for_scenario(assertion, trace_id)
            results["scenario-preconditions"] = TestResult(
                TestStatus.PASSED,
                "raw calls and command actions prove main/subagent topology",
            )

            test_context.report_progress(
                "export_flush",
                "finishing buffered export and polling for target requests",
            )
            self._environment.finish_buffered_export()
            spans = self._wait_for_request_spans(scenario.trace_name, evidence)

            test_context.report_progress(
                "trajectory_assertions",
                "validating isolated continuations across action, API, and OTel",
            )
            trajectory_ids = assertion.require_trajectory(evidence)
            assertion.require_otel(evidence, trajectory_ids, spans)
            results["product-assertions"] = TestResult(
                TestStatus.PASSED,
                "Claude main/subagent trajectories are isolated and consistent",
            )
            return TestResult(
                TestStatus.COMPOSITE,
                "Claude subagent LLM trajectory regression",
                results,
            )
        except ScenarioPreconditionFailure as error:
            results["scenario-precondition-failure"] = TestResult(
                TestStatus.FAILED,
                str(error),
            )
        except ProductAssertionFailure as error:
            results["product-assertion-failure"] = TestResult(
                TestStatus.FAILED,
                str(error),
            )
        except Exception as error:
            results["infrastructure-failure"] = TestResult(
                TestStatus.FAILED,
                str(error),
            )
        return TestResult(
            TestStatus.COMPOSITE,
            "Claude subagent LLM trajectory regression",
            results,
        )
