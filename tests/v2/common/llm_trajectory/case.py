from __future__ import annotations

import json
import time
from typing import Any, Protocol, TypeVar

from tests.v2.common.core import TestCase, TestResult
from tests.v2.common.runner import TestingContextSingleton

from .assertion import ScenarioPreconditionFailure
from .config import TrajectoryTestConfig
from .environment import TrajectoryTestEnvironment

EvidenceT = TypeVar("EvidenceT")


class ScenarioAssertion(Protocol[EvidenceT]):
    def require_scenario(self, document: dict[str, Any]) -> EvidenceT: ...


class TrajectoryCaseSupport(TestCase):
    def __init__(self, config: TrajectoryTestConfig):
        self._config = config
        self._environment: TrajectoryTestEnvironment | None = None

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
        assertion: ScenarioAssertion[EvidenceT],
        trace_id: int,
    ) -> EvidenceT:
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
