from __future__ import annotations

import time
from typing import Any

from tests.v2.common.core import TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton
from tests.v2.common.testing_env import AgentBinaryDiscovery
from tests.v2.common.llm_trajectory.assertion import (
    ProductAssertionFailure,
    ScenarioPreconditionFailure,
)
from tests.v2.common.llm_trajectory.case import TrajectoryCaseSupport
from tests.v2.common.llm_trajectory.environment import (
    TrajectoryTestEnvironment,
)

from .assertion import ProjectSubagentEvidence, ProjectSubagentTrajectoryAssertion
from .agent import ProjectSubagentAgent
from .config import ProjectSubagentTrajectoryConfig
from .scenario import ProjectSubagentTrajectoryScenario


class ProjectSubagentTrajectoryCase(TrajectoryCaseSupport):
    _TRAJECTORY_ID = "llm.request.trajectory_id"

    def __init__(self, config: ProjectSubagentTrajectoryConfig):
        super().__init__(config)
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        try:
            discovery = AgentBinaryDiscovery(self._config.repo)
            agent = ProjectSubagentAgent.resolve(
                self._config.agent_binary,
                discovery,
            )
            if agent is None:
                return TestResult(
                    TestStatus.SKIPPED,
                    f"{self._config.agent_binary} executable not found; set "
                    f"{self._config.agent_binary.upper()}_E2E_BINARY",
                )
            test_context.report_progress(
                "agent_availability",
                f"checking {agent.name} availability before starting capture",
            )
            if not test_context.check_agent_availability(
                agent.name,
                agent.binary,
                agent.environment,
            ):
                return TestResult(
                    TestStatus.SKIPPED,
                    f"{agent.name} external availability check failed",
                )

            test_context.report_progress(
                "environment_prepare",
                "starting refreshed daemon, Web API, and OTel HTTP receiver",
            )
            self._environment = TrajectoryTestEnvironment(
                self._config,
                test_context.output,
            )
            self._environment.prepare()
            self._environment.configure_buffered_export(
                {"llm.request", "llm.response"}
            )
            results["infrastructure"] = TestResult(
                TestStatus.PASSED,
                "daemon and complete request/response export are ready",
            )

            scenario = ProjectSubagentTrajectoryScenario(
                self._environment.runtime,
                agent,
                self._config.repo,
                self._config.launch_timeout_seconds,
                self._config.trace_random_bytes,
            )
            test_context.report_progress(
                "agent_launch",
                f"running {agent.name} with three project-inspection subagents",
            )
            scenario.run()
            trace_id = self._wait_for_trace_id(scenario.trace_name)
            self._wait_for_terminal_trace(trace_id)
            results["agent-run"] = TestResult(
                TestStatus.PASSED,
                f"trace-{trace_id} completed the delegated run",
            )

            assertion = ProjectSubagentTrajectoryAssertion(
                self._environment,
                trace_id,
            )
            test_context.report_progress(
                "projection_assertions",
                "proving delegation, global call pairing, and trajectory topology",
            )
            evidence = self._wait_for_scenario(assertion, trace_id)
            trajectory_ids = assertion.require_trajectories(evidence)
            results["semantic-projection"] = TestResult(
                TestStatus.PASSED,
                f"{len(evidence.pairs)} complete exchanges have consistent "
                f"trajectory lineage for {agent.name}",
            )

            test_context.report_progress(
                "export_flush",
                "flushing and verifying every request and response export",
            )
            self._environment.finish_buffered_export()
            spans = self._wait_for_all_llm_spans(
                scenario.trace_name,
                evidence,
            )
            self._require_all_llm_spans(evidence, trajectory_ids, spans)
            results["otel-export"] = TestResult(
                TestStatus.PASSED,
                "every paired request and response was exported exactly once",
            )
            return TestResult(
                TestStatus.COMPOSITE,
                f"{agent.name} project subagent LLM projection regression",
                results,
            )
        except (ScenarioPreconditionFailure, ProductAssertionFailure) as error:
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
            f"{self._config.agent_binary} project subagent LLM projection regression",
            results,
        )

    def _wait_for_trace_id(self, trace_name: str) -> int:
        last_matches: list[dict[str, Any]] = []
        for _ in range(self._config.drain_attempts):
            document = self._viewer_json(["traces"])
            traces = document.get("traces")
            if not isinstance(traces, list):
                raise RuntimeError("actrailviewer traces returned no traces array")
            last_matches = [
                trace
                for trace in traces
                if isinstance(trace, dict) and trace.get("name") == trace_name
            ]
            if len(last_matches) == 1:
                trace_id = last_matches[0].get("trace_id_raw")
                if isinstance(trace_id, int):
                    return trace_id
                raise RuntimeError(f"trace {trace_name} has no numeric trace id")
            time.sleep(self._config.drain_interval_seconds)
        raise RuntimeError(
            f"expected one persisted trace named {trace_name}, found {len(last_matches)}"
        )

    def _wait_for_all_llm_spans(
        self,
        trace_name: str,
        evidence: ProjectSubagentEvidence,
    ) -> list[dict[str, Any]]:
        required_ids = {
            str(action["action_id"])
            for pair in evidence.pairs
            for action in (pair.request, pair.response)
        }
        observed_ids: set[str] = set()
        spans: list[dict[str, Any]] = []
        for _ in range(self._config.drain_attempts):
            spans = self._marker_spans(trace_name)
            observed_ids = {
                action_id
                for span in spans
                if (action_id := self._otel_attribute(span, "actrail.action.id"))
            }
            if required_ids.issubset(observed_ids):
                return spans
            time.sleep(self._config.drain_interval_seconds)
        missing = sorted(required_ids.difference(observed_ids))
        raise ProductAssertionFailure(
            "OTel HTTP did not export all paired LLM actions; missing="
            + ", ".join(missing)
        )

    def _require_all_llm_spans(
        self,
        evidence: ProjectSubagentEvidence,
        trajectory_ids: dict[str, str],
        spans: list[dict[str, Any]],
    ) -> None:
        spans_by_action: dict[str, list[dict[str, Any]]] = {}
        for span in spans:
            action_id = self._otel_attribute(span, "actrail.action.id")
            if action_id:
                spans_by_action.setdefault(action_id, []).append(span)
        for pair in evidence.pairs:
            request_id = str(pair.request["action_id"])
            response_id = str(pair.response["action_id"])
            self._require_one_span(spans_by_action, request_id, "llm.request")
            self._require_one_span(spans_by_action, response_id, "llm.response")
            request_span = spans_by_action[request_id][0]
            if self._otel_attribute(
                request_span,
                self._TRAJECTORY_ID,
            ) != trajectory_ids[request_id]:
                raise ProductAssertionFailure(
                    f"OTel request {request_id} has inconsistent trajectory id"
                )

    def _require_one_span(
        self,
        spans_by_action: dict[str, list[dict[str, Any]]],
        action_id: str,
        expected_kind: str,
    ) -> None:
        matches = spans_by_action.get(action_id, [])
        if len(matches) != 1:
            raise ProductAssertionFailure(
                f"OTel expected one {expected_kind} span for {action_id}, "
                f"found {len(matches)}"
            )
        if self._otel_attribute(matches[0], "actrail.action.kind") != expected_kind:
            raise ProductAssertionFailure(
                f"OTel action {action_id} is not {expected_kind}"
            )
