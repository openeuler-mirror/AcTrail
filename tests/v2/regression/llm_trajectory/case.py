from __future__ import annotations

import json
import time
from pathlib import Path
from typing import Any

from tests.v2.common.core import TestCase, TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton
from tests.v2.common.testing_env import AgentBinaryDiscovery

from .assertion import (
    LlmTrajectoryAssertion,
    ProductAssertionFailure,
    ScenarioPreconditionFailure,
    TrajectoryEvidence,
)
from .config import LlmTrajectoryConfig
from .environment import LlmTrajectoryEnvironment
from .scenario import FixtureRepository, OpenCodeSubagentScenario


class LlmTrajectoryCase(TestCase):
    def __init__(self, config: LlmTrajectoryConfig):
        self._config = config
        self._environment: LlmTrajectoryEnvironment | None = None

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        try:
            discovery = AgentBinaryDiscovery(self._config.repo)
            opencode = discovery.resolve("OPENCODE_E2E_BINARY", "opencode")
            if opencode is None:
                return TestResult(
                    TestStatus.SKIPPED,
                    "OpenCode executable not found; set OPENCODE_E2E_BINARY",
                )
            opencode_environment = discovery.environment(opencode)
            test_context.report_progress(
                "agent_availability",
                "checking OpenCode availability before starting capture",
            )
            if not test_context.check_agent_availability(
                "opencode",
                opencode,
                opencode_environment,
            ):
                return TestResult(
                    TestStatus.SKIPPED,
                    "OpenCode external availability check failed",
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
                "daemon, Web API, and metadata-only llm.request exporter are ready",
            )

            fixture = FixtureRepository.create(
                self._environment.runtime,
                self._config.work_dir,
            )
            scenario = OpenCodeSubagentScenario(
                self._environment.runtime,
                opencode,
                opencode_environment,
                fixture,
                self._config.launch_timeout_seconds,
            )
            test_context.report_progress(
                "agent_launch",
                "running OpenCode with a mandatory general subagent git task",
            )
            trace_id, launch = scenario.run()
            if scenario.answer_marker not in launch.stdout:
                raise ScenarioPreconditionFailure(
                    f"OpenCode final output has no answer marker {scenario.answer_marker}"
                )
            if fixture.commit_id not in launch.stdout:
                raise ScenarioPreconditionFailure(
                    "OpenCode final output did not report the fixture commit"
                )
            self._wait_for_terminal_trace(trace_id)
            results["agent-run"] = TestResult(
                TestStatus.PASSED,
                f"real OpenCode trace-{trace_id} returned the fixture commit",
            )

            assertion = LlmTrajectoryAssertion(
                self._environment,
                trace_id,
                scenario.task_marker,
                fixture.commit_id,
                self._config.request_content_max_bytes,
            )
            test_context.report_progress(
                "scenario_preconditions",
                "proving title, executed subagent task, git tool, and both continuations",
            )
            evidence = self._wait_for_scenario(assertion, trace_id)
            results["scenario-preconditions"] = TestResult(
                TestStatus.PASSED,
                "raw requests, responses, links, and command actions prove the expected topology",
            )

            test_context.report_progress(
                "export_flush",
                "finishing the buffered exporter and polling for all target requests",
            )
            self._environment.finish_buffered_export()
            spans = self._wait_for_request_spans(scenario.trace_name, evidence)

            test_context.report_progress(
                "trajectory_assertions",
                "validating lineage, strict-prefix continuations, isolation, and OTel IDs",
            )
            trajectory_ids = assertion.require_trajectory(evidence)
            assertion.require_otel(evidence, trajectory_ids, spans)
            results["product-assertions"] = TestResult(
                TestStatus.PASSED,
                "title/main/subagent trajectories are isolated, continued, and exported",
            )
            return TestResult(
                TestStatus.COMPOSITE,
                "OpenCode subagent LLM trajectory regression",
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
            "OpenCode subagent LLM trajectory regression",
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

    def _wait_for_terminal_trace(self, trace_id: int) -> None:
        last_state = "missing"
        for _ in range(self._config.drain_attempts):
            document = self._viewer_json(["traces"])
            traces = document.get("traces")
            if not isinstance(traces, list):
                raise RuntimeError("actrailviewer traces returned no traces array")
            for trace in traces:
                if not isinstance(trace, dict) or trace.get("trace_id_raw") != trace_id:
                    continue
                last_state = f"{trace.get('state')}/{trace.get('health')}"
                if last_state in {"Exited/Clean", "Completed/Clean"}:
                    return
            time.sleep(self._config.drain_interval_seconds)
        raise RuntimeError(
            f"trace-{trace_id} did not reach a clean terminal state; last={last_state}"
        )

    def _wait_for_scenario(
        self,
        assertion: LlmTrajectoryAssertion,
        trace_id: int,
    ) -> TrajectoryEvidence:
        last_error: ScenarioPreconditionFailure | None = None
        for _ in range(self._config.drain_attempts):
            try:
                return assertion.require_scenario(
                    self._viewer_json(["actions", "--trace-id", str(trace_id)])
                )
            except ScenarioPreconditionFailure as error:
                last_error = error
                time.sleep(self._config.drain_interval_seconds)
        raise ScenarioPreconditionFailure(
            "scenario precondition failed after polling: "
            f"{last_error or 'no evidence'}"
        )

    def _wait_for_request_spans(
        self,
        trace_name: str,
        evidence: TrajectoryEvidence,
    ) -> list[dict[str, Any]]:
        required_ids = {
            pair.request["action_id"] for _, pair in evidence.request_pairs
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
            "OTel HTTP did not export all target llm.request actions; missing="
            + ", ".join(missing)
        )

    def _marker_spans(self, trace_name: str) -> list[dict[str, Any]]:
        assert self._environment is not None
        spans: list[dict[str, Any]] = []
        for document in self._environment.documents():
            for resource_spans in document.get("resourceSpans", []):
                if not isinstance(resource_spans, dict):
                    continue
                resource = resource_spans.get("resource", {})
                if self._otel_attribute(
                    resource,
                    "actrail.trace.display_name",
                ) != trace_name:
                    continue
                for scope_spans in resource_spans.get("scopeSpans", []):
                    if not isinstance(scope_spans, dict):
                        continue
                    spans.extend(
                        span
                        for span in scope_spans.get("spans", [])
                        if isinstance(span, dict)
                    )
        return spans

    def _viewer_json(self, arguments: list[str]) -> dict[str, Any]:
        assert self._environment is not None
        result = self._environment.runtime.run(
            [
                *self._environment.runtime.viewer_command(
                    "--output-format",
                    "json",
                ),
                *arguments,
            ],
            echo=False,
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"actrailviewer {' '.join(arguments)} exited with "
                f"{result.returncode}: {result.stderr[-2000:]}"
            )
        try:
            document = json.loads(result.stdout)
        except json.JSONDecodeError as error:
            raise RuntimeError("actrailviewer returned invalid JSON") from error
        if not isinstance(document, dict):
            raise RuntimeError("actrailviewer returned non-object JSON")
        return document

    @staticmethod
    def _otel_attribute(container: Any, key: str) -> str | None:
        if not isinstance(container, dict):
            return None
        attributes = container.get("attributes")
        if not isinstance(attributes, list):
            return None
        for attribute in attributes:
            if not isinstance(attribute, dict) or attribute.get("key") != key:
                continue
            value = attribute.get("value")
            if not isinstance(value, dict):
                return None
            string_value = value.get("stringValue")
            if isinstance(string_value, str):
                return string_value
        return None
