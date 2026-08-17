from __future__ import annotations

import json
import re
import secrets
import sqlite3
import time
from collections import Counter
from pathlib import Path
from typing import Any

from tests.v2.common.core import TestCase, TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton
from tests.v2.common.testing_env import AgentBinaryDiscovery

from .config import OtelHttpConfig
from .environment import OtelHttpEnvironment


class OtelHttpCase(TestCase):
    _TRACE_PATTERN = re.compile(r"trace trace-(\d+) entered Active")
    _TERMINAL_STATUSES = {"success", "error", "unknown"}
    _METADATA_KEYS = {
        "actrail.action.id",
        "actrail.action.kind",
        "actrail.action.status",
        "actrail.action.completeness",
        "actrail.process.id",
        "actrail.action.valid",
        "process.parent.identity_state",
    }

    def __init__(self, config: OtelHttpConfig):
        self._config = config
        self._environment: OtelHttpEnvironment | None = None

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        try:
            discovery = AgentBinaryDiscovery(self._config.repo)
            xiaoo = discovery.resolve("XIAOO_E2E_BINARY", "xiaoo")
            if xiaoo is None:
                return TestResult(
                    TestStatus.SKIPPED,
                    "xiaoO executable not found; set XIAOO_E2E_BINARY to its path",
                )
            xiaoo_environment = discovery.environment(xiaoo)
            test_context.report_progress(
                "agent_availability",
                "checking xiaoO availability",
            )
            if not test_context.check_agent_availability(
                "xiaoo", xiaoo, xiaoo_environment
            ):
                return TestResult(
                    TestStatus.SKIPPED,
                    "xiaoO external availability check failed",
                )

            test_context.report_progress(
                "environment_prepare",
                "starting actraild, actrailweb, otel-http, and local OTLP receiver",
            )
            self._environment = OtelHttpEnvironment(self._config, test_context.output)
            self._environment.prepare()
            results["plugin-load"] = TestResult(
                TestStatus.PASSED,
                "builtin otel-http loaded with editable safe-default schema",
            )

            self._environment.configure_buffered_export()
            results["outbound-policy"] = TestResult(
                TestStatus.PASSED,
                "explicit process action allow-list and metadata-only mode accepted",
            )

            marker = f"OTEL_HTTP_V2_{secrets.token_hex(8)}"
            trace_id = self._launch(marker, xiaoo, xiaoo_environment)
            self._wait_for_terminal_trace(trace_id)
            results["source-trace"] = TestResult(
                TestStatus.PASSED,
                f"real xiaoO trace-{trace_id} reached a clean terminal state",
            )

            if self._marker_spans(marker):
                raise AssertionError(
                    "OTEL/HTTP flushed before batch limits or lifecycle finish"
                )
            self._environment.finish_buffered_export()
            spans = self._wait_for_marker_spans(marker)
            results["shutdown-tail"] = TestResult(
                TestStatus.PASSED,
                f"lifecycle finish flushed {len(spans)} buffered span(s)",
            )

            counts = self._require_terminal_one_shot(spans)
            results["terminal-actions"] = TestResult(
                TestStatus.PASSED,
                "terminal one-shot counts: "
                + ", ".join(
                    f"{kind}={count}" for kind, count in sorted(counts.items())
                ),
            )
            self._require_metadata_only(spans)
            results["metadata-only"] = TestResult(
                TestStatus.PASSED,
                "all exported span attributes stayed within structural metadata",
            )
            otel_trace_id = self._require_persistent_external_identity(trace_id, spans)
            results["external-trace-id"] = TestResult(
                TestStatus.PASSED,
                f"OTLP traceId {otel_trace_id} matches the persisted 16-byte identity",
            )
            requests = self._require_configured_credential()
            results["request-credential"] = TestResult(
                TestStatus.PASSED,
                f"configured credential header reached the receiver on {requests} request(s)",
            )
            return TestResult(
                TestStatus.COMPOSITE,
                "builtin OTEL/HTTP V2 boundary regression",
                results,
            )
        except Exception as error:
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
            return TestResult(
                TestStatus.COMPOSITE,
                "builtin OTEL/HTTP V2 boundary regression",
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

    def _launch(
        self,
        marker: str,
        xiaoo: Path,
        environment: dict[str, str],
    ) -> int:
        assert self._environment is not None
        answer_marker = f"A{secrets.token_hex(5)}"
        result = self._environment.runtime.run(
            [
                *self._environment.runtime.control_command("launch"),
                "--name",
                marker,
                "--host-ebpf",
                "required",
                "--seccomp-notify",
                "auto",
                "--",
                xiaoo,
                "--cli",
                "run",
                "--no-tools",
                "--max-turns",
                "1",
                "--prompt",
                f'Reply with exactly "{answer_marker}" and nothing else. Do not use tools.',
            ],
            timeout_seconds=self._config.launch_timeout_seconds,
            environment=environment,
        )
        if result.returncode != 0:
            raise AssertionError(
                f"actrailctl launch exited with {result.returncode}: "
                f"{result.output[-4000:]}"
            )
        if answer_marker not in result.stdout:
            raise AssertionError(
                f"xiaoO output did not contain answer marker {answer_marker}: "
                f"{result.output[-4000:]}"
            )
        trace_ids = [int(value) for value in self._TRACE_PATTERN.findall(result.output)]
        if len(trace_ids) != 1:
            raise AssertionError(
                f"expected one trace id, found {trace_ids}: {result.output[-4000:]}"
            )
        return trace_ids[0]

    def _require_persistent_external_identity(
        self,
        local_trace_id: int,
        spans: list[dict[str, Any]],
    ) -> str:
        wire_ids = {span.get("traceId") for span in spans}
        if len(wire_ids) != 1:
            raise AssertionError(f"OTLP spans do not share one traceId: {wire_ids}")
        wire_id = next(iter(wire_ids))
        if not isinstance(wire_id, str) or re.fullmatch(r"[0-9a-f]{32}", wire_id) is None:
            raise AssertionError(f"OTLP traceId is not 32 lowercase hex digits: {wire_id!r}")
        if wire_id == "0" * 32:
            raise AssertionError("OTLP traceId must not be all zero")
        if wire_id == f"{local_trace_id:032x}":
            raise AssertionError("OTLP traceId still contains the widened local u64 id")
        if wire_id[12] != "4" or wire_id[16] not in "89ab":
            raise AssertionError(f"OTLP traceId is not an RFC 4122 UUIDv4 value: {wire_id}")

        database = self._config.work_dir / "data" / "actrail.sqlite"
        with sqlite3.connect(database) as connection:
            row = connection.execute(
                "SELECT lower(hex(otel_trace_id)), length(otel_trace_id) "
                "FROM traces WHERE trace_id = ?",
                (local_trace_id,),
            ).fetchone()
        if row != (wire_id, 16):
            raise AssertionError(
                "OTLP traceId does not match SQLite identity: "
                f"wire={wire_id}, sqlite={row!r}"
            )
        return wire_id

    def _wait_for_terminal_trace(self, trace_id: int) -> None:
        last_state = "<missing>"
        for _ in range(self._config.drain_attempts):
            document = self._viewer_json(["traces"])
            traces = document.get("traces")
            if not isinstance(traces, list):
                raise AssertionError("actrailviewer traces returned no traces array")
            for trace in traces:
                if (
                    isinstance(trace, dict)
                    and trace.get("trace_id_raw") == trace_id
                ):
                    last_state = f"{trace.get('state')}/{trace.get('health')}"
                    if last_state in {"Exited/Clean", "Completed/Clean"}:
                        # process.exit is an export-only action, so terminal trace
                        # state is the observable storage-side synchronization point.
                        time.sleep(self._config.drain_interval_seconds)
                        return
            time.sleep(self._config.drain_interval_seconds)
        raise AssertionError(
            f"trace-{trace_id} did not reach a clean terminal state; last={last_state}"
        )

    def _wait_for_marker_spans(self, marker: str) -> list[dict[str, Any]]:
        spans: list[dict[str, Any]] = []
        for _ in range(self._config.drain_attempts):
            spans = self._marker_spans(marker)
            kinds = {self._attribute(span, "actrail.action.kind") for span in spans}
            if {"process.exec", "process.exit"}.issubset(kinds):
                return spans
            time.sleep(self._config.drain_interval_seconds)
        raise AssertionError(
            "OTEL/HTTP lifecycle finish did not flush process.exec and process.exit; "
            f"observed={sorted(kind for kind in kinds if kind)}"
        )

    def _marker_spans(self, marker: str) -> list[dict[str, Any]]:
        assert self._environment is not None
        spans: list[dict[str, Any]] = []
        for document in self._environment.documents():
            for resource_spans in document.get("resourceSpans", []):
                if not isinstance(resource_spans, dict):
                    continue
                resource = resource_spans.get("resource", {})
                if self._attribute(resource, "actrail.trace.display_name") != marker:
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

    def _require_terminal_one_shot(
        self,
        spans: list[dict[str, Any]],
    ) -> Counter[str]:
        action_ids: list[str] = []
        kinds: list[str] = []
        invalid_statuses: list[str | None] = []
        for span in spans:
            action_id = self._attribute(span, "actrail.action.id")
            kind = self._attribute(span, "actrail.action.kind")
            status = self._attribute(span, "actrail.action.status")
            if not action_id or not kind:
                raise AssertionError("OTEL/HTTP span has no action id or kind")
            action_ids.append(action_id)
            kinds.append(kind)
            if status not in self._TERMINAL_STATUSES:
                invalid_statuses.append(status)
        duplicates = sorted(
            action_id for action_id, count in Counter(action_ids).items() if count != 1
        )
        if duplicates:
            raise AssertionError(
                "OTEL/HTTP emitted duplicate revisions for action id(s): "
                + ", ".join(duplicates)
            )
        if invalid_statuses:
            raise AssertionError(
                f"OTEL/HTTP emitted non-terminal status(es): {invalid_statuses}"
            )
        counts = Counter(kinds)
        if not {"process.exec", "process.exit"}.issubset(counts):
            raise AssertionError(f"OTEL/HTTP action kinds are incomplete: {counts}")
        return counts

    def _require_configured_credential(self) -> int:
        """Every accepted request must carry the configured credential header.

        A receiver that attributes traces by request header sees unattributable
        data if the exporter drops it, even though the POST still returns 200.
        """
        assert self._environment is not None
        credentials = self._environment.credentials()
        if not credentials:
            raise AssertionError("receiver accepted no request to check")
        unexpected = [
            value
            for value in credentials
            if value != OtelHttpEnvironment.EXPORT_CREDENTIAL
        ]
        if unexpected:
            raise AssertionError(
                f"{len(unexpected)} of {len(credentials)} request(s) carried "
                f"credential {unexpected[0]!r}, expected "
                f"{OtelHttpEnvironment.EXPORT_CREDENTIAL!r}"
            )
        return len(credentials)

    def _require_metadata_only(self, spans: list[dict[str, Any]]) -> None:
        unexpected: set[str] = set()
        for span in spans:
            kind = self._attribute(span, "actrail.action.kind")
            if span.get("name") != kind:
                raise AssertionError(
                    "metadata-only OTEL/HTTP span name exposed its source title: "
                    f"name={span.get('name')!r}, kind={kind!r}"
                )
            attributes = span.get("attributes")
            if not isinstance(attributes, list):
                raise AssertionError("OTEL/HTTP span has no attributes array")
            for attribute in attributes:
                if not isinstance(attribute, dict):
                    raise AssertionError("OTEL/HTTP span has malformed attribute")
                key = attribute.get("key")
                if not isinstance(key, str):
                    raise AssertionError("OTEL/HTTP span attribute has no key")
                if key not in self._METADATA_KEYS:
                    unexpected.add(key)
        if unexpected:
            raise AssertionError(
                "metadata-only OTEL/HTTP exported content attribute(s): "
                + ", ".join(sorted(unexpected))
            )

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
            raise AssertionError(
                f"actrailviewer {' '.join(arguments)} exited with "
                f"{result.returncode}: {result.stderr[-2000:]}"
            )
        try:
            document = json.loads(result.stdout)
        except json.JSONDecodeError as error:
            raise AssertionError("actrailviewer returned invalid JSON") from error
        if not isinstance(document, dict):
            raise AssertionError("actrailviewer returned non-object JSON")
        return document

    @staticmethod
    def _attribute(container: Any, key: str) -> str | None:
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
            int_value = value.get("intValue")
            if isinstance(int_value, (str, int)):
                return str(int_value)
        return None
