from __future__ import annotations

import hashlib
import json
import re
import secrets
import sqlite3
import time
from collections import Counter
from pathlib import Path
from typing import Any

from tests.e2e.common.core import TestCase, TestResult, TestStatus
from tests.e2e.common.runner import TestingContextSingleton
from tests.e2e.common.testing_env import AgentBinaryDiscovery

from .config import OtelHttpConfig
from .commands import CommandSpanVerification
from .environment import OtelHttpEnvironment
from .tool_agents import TOOL_AGENTS, ToolAgent, ToolChainTask, resolve_tool_agent

from .constants import CASE_CONSTANTS


class OtelHttpCase(TestCase):
    _ACTION_KINDS = {
        "process.exec", "process.exit", "agent.identity", "llm.request",
        "command.invocation",
    }
    _TRACE_PATTERN = re.compile(r"trace trace-(\d+) entered Active")
    _TERMINAL_STATUSES = {"success", "error", "unknown"}
    _TOOL_ROUND_KINDS = {"process.exec", "process.exit", "llm.tool_call", "llm.tool_result"}
    _METADATA_KEYS = {
        "actrail.action.id",
        "actrail.action.kind",
        "actrail.action.status",
        "actrail.action.completeness",
        "actrail.process.id",
        "actrail.action.valid",
        "process.parent.identity_state",
        "process.executable",
        "agent.identity.status",
        "agent.identity.source",
        "agent.identity.evidence_action_id",
        "actrail.agent.type",
        "actrail.agent.type.source",
        "llm.request.trajectory_id",
        "llm.request.trajectory_inference_version",
        "llm.tool_call.id",
    }
    _SHORT_TOOL_CALL_ID_HEX = 12

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

            self._environment.configure_buffered_export(
                self._ACTION_KINDS
            )
            results["outbound-policy"] = TestResult(
                TestStatus.PASSED,
                "explicit process/identity/request/command allow-list and metadata-only mode accepted",
            )

            commands = CommandSpanVerification(self._config)
            marker = f"OTEL_HTTP_V2_{secrets.token_hex(CASE_CONSTANTS.marker_random_bytes)}"
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
            spans = self._wait_for_marker_spans(
                marker,
                self._ACTION_KINDS,
            )
            results["shutdown-tail"] = TestResult(
                TestStatus.PASSED,
                f"lifecycle finish flushed {len(spans)} buffered span(s)",
            )

            counts = self._require_terminal_one_shot(spans)
            if counts["llm.request"] < 1:
                raise AssertionError(
                    "metadata-only OTEL/HTTP round exported no llm.request span"
                )
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
                "llm.request and process spans stayed within structural metadata",
            )
            otel_trace_id = self._require_persistent_external_identity(trace_id, spans)
            results["external-trace-id"] = TestResult(
                TestStatus.PASSED,
                f"OTLP traceId {otel_trace_id} matches the persisted 16-byte identity",
            )

            identity_binary = self._require_agent_identity_correlation(
                spans,
                otel_trace_id,
                xiaoo,
            )
            results["agent-identity-correlation"] = TestResult(
                TestStatus.PASSED,
                f"agent.identity binds {identity_binary} and its resolved "
                "agent type to OTLP traceId",
            )

            self._environment.configure_buffered_export(
                self._ACTION_KINDS,
                attribute_mode="full",
            )
            body_marker = f"REQUEST_BODY_V2_{secrets.token_hex(CASE_CONSTANTS.marker_random_bytes)}"
            full_marker = f"OTEL_HTTP_FULL_V2_{secrets.token_hex(CASE_CONSTANTS.marker_random_bytes)}"
            full_trace_id = self._launch(
                full_marker,
                xiaoo,
                xiaoo_environment,
                request_marker=body_marker,
            )
            self._wait_for_terminal_trace(full_trace_id)
            if self._marker_spans(full_marker):
                raise AssertionError(
                    "full OTEL/HTTP round flushed before lifecycle finish"
                )
            self._environment.finish_buffered_export()
            full_spans = self._wait_for_marker_spans(
                full_marker,
                self._ACTION_KINDS,
            )
            full_counts = self._require_terminal_one_shot(full_spans)
            if full_counts["llm.request"] < 1:
                raise AssertionError("full OTEL/HTTP round exported no llm.request span")
            exported_bodies = self._require_exported_request_body(
                full_spans,
                body_marker,
            )
            results["request-body-full-export"] = TestResult(
                TestStatus.PASSED,
                f"full mode exported {exported_bodies} canonical request body/bodies",
            )

            for mode, local_id, received in (
                ("metadata-only", trace_id, spans),
                ("full", full_trace_id, full_spans),
            ):
                try:
                    summary = commands.verify(
                        local_id, mode, received, self._ACTION_KINDS,
                    )
                    results[f"command-terminal-{mode}"] = TestResult(
                        TestStatus.PASSED, summary
                    )
                except AssertionError as error:
                    results[f"command-terminal-{mode}"] = TestResult(
                        TestStatus.FAILED, str(error)
                    )

            results.update(self._tool_result_rounds(test_context))

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

    def _tool_result_rounds(
        self,
        test_context: TestingContextSingleton,
    ) -> dict[str, TestResult]:
        """Each installed agent runs a two-file task; every tool call needs one bound result."""
        discovery = AgentBinaryDiscovery(self._config.repo)
        results: dict[str, TestResult] = {}
        available: list[ToolAgent] = []
        for kind, variable, executable in TOOL_AGENTS:
            name = f"tool-results-{kind}"
            agent = resolve_tool_agent(discovery, kind, variable, executable)
            if agent is None:
                results[name] = TestResult(
                    TestStatus.SKIPPED,
                    f"{kind} executable not found; set {variable} to its path",
                )
                continue
            test_context.report_progress(
                "agent_availability",
                f"checking {kind} availability",
            )
            if not test_context.check_agent_availability(
                kind, agent.binary, agent.environment
            ):
                results[name] = TestResult(
                    TestStatus.SKIPPED,
                    f"{kind} external availability check failed",
                )
                continue
            available.append(agent)
            test_context.report_progress(
                "tool_results",
                f"exporting tool calls and results of {kind}",
            )
            try:
                results[name] = TestResult(
                    TestStatus.PASSED,
                    self._require_tool_results_with_calls(agent),
                )
            except AssertionError as error:
                results[name] = TestResult(TestStatus.FAILED, f"{kind}: {error}")
        results["tool-results-metadata-only"] = self._metadata_only_tool_round(
            test_context, available
        )
        return results

    def _metadata_only_tool_round(
        self,
        test_context: TestingContextSingleton,
        available: list[ToolAgent],
    ) -> TestResult:
        """The default export mode still pairs each tool call with its result."""
        agent = next((agent for agent in available if agent.kind == "xiaoo"), None)
        agent = agent or (available[0] if available else None)
        if agent is None:
            return TestResult(TestStatus.SKIPPED, "no tool agent is available")
        test_context.report_progress(
            "tool_results",
            f"pairing tool calls and results of {agent.kind} in metadata-only mode",
        )
        try:
            spans = self._run_tool_chain(agent, "metadata-only")
            self._require_metadata_only(spans)
            call_ids = self._paired_tool_ids(spans)
        except AssertionError as error:
            return TestResult(TestStatus.FAILED, f"{agent.kind}: {error}")
        return TestResult(
            TestStatus.PASSED,
            f"{agent.kind}: {len(call_ids)} tool call(s) paired with their result "
            "by llm.tool_call.id without content attributes",
        )

    def _require_tool_results_with_calls(self, agent: ToolAgent) -> str:
        spans = self._run_tool_chain(agent, "full")
        call_ids = self._paired_tool_ids(spans)
        mismatched = self._results_without_their_call(spans)
        if mismatched:
            raise AssertionError(f"tool result(s) do not carry their call: {mismatched}")
        unshortened = self._calls_without_short_id(spans)
        if unshortened:
            raise AssertionError(f"tool call id is not the short form of the protocol id: {unshortened}")
        # The parent span already names the response or request; the id is not repeated.
        repeated = sorted(
            key
            for span in spans
            for key in ("llm.tool_call.response_action_id", "llm.tool_result.request_action_id")
            if self._attribute(span, key) is not None
        )
        if repeated:
            raise AssertionError(f"tool span repeated its parent id: {repeated}")
        return f"{len(call_ids)} tool call(s), each with one tool result carrying the call"

    def _run_tool_chain(self, agent: ToolAgent, attribute_mode: str) -> list[dict[str, Any]]:
        """Run the two-file task and return its exported spans."""
        assert self._environment is not None
        self._environment.configure_buffered_export(
            self._TOOL_ROUND_KINDS,
            attribute_mode=attribute_mode,
        )
        # Each round needs its own files; the metadata-only round reuses an agent.
        suffix = "" if attribute_mode == "full" else f"-{attribute_mode}"
        task = ToolChainTask.create(self._config.work_dir / f"tool-chain-{agent.kind}{suffix}")
        marker = (
            f"OTEL_HTTP_TOOLS_V2_{agent.kind}_"
            f"{secrets.token_hex(CASE_CONSTANTS.marker_random_bytes)}"
        )
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
                *agent.command(task.prompt),
            ],
            timeout_seconds=self._config.launch_timeout_seconds,
            # Agents such as OpenCode reject reads outside their working directory,
            # which OpenCode takes from PWD rather than from the process.
            environment={**agent.environment, "PWD": str(task.first.parent)},
            cwd=task.first.parent,
        )
        tail = result.output[-CASE_CONSTANTS.launch_diagnostic_tail_characters:]
        if result.returncode != 0:
            raise AssertionError(f"actrailctl launch exited with {result.returncode}: {tail}")
        if task.answer not in result.stdout:
            raise AssertionError(f"output did not contain the second file's text: {tail}")
        trace_ids = [int(value) for value in self._TRACE_PATTERN.findall(result.output)]
        if len(trace_ids) != 1:
            raise AssertionError(f"expected one trace id, found {trace_ids}")
        self._wait_for_terminal_trace(trace_ids[0])
        self._environment.finish_buffered_export()
        return self._wait_for_marker_spans(marker, {"process.exit", "llm.tool_call"})

    def _paired_tool_ids(self, spans: list[dict[str, Any]]) -> list[str]:
        """Every tool call has exactly one result with the same llm.tool_call.id."""
        call_ids = self._tool_ids(spans, "llm.tool_call", "llm.tool_call.id")
        result_ids = self._tool_ids(spans, "llm.tool_result", "llm.tool_call.id")
        if Counter(result_ids) != Counter(call_ids):
            missing = sorted((Counter(call_ids) - Counter(result_ids)).elements())
            extra = sorted((Counter(result_ids) - Counter(call_ids)).elements())
            raise AssertionError(
                f"{len(call_ids)} tool call(s) but {len(result_ids)} tool result(s); "
                f"calls without a result={missing}, unexpected results={extra}"
            )
        return call_ids

    def _calls_without_short_id(self, spans: list[dict[str, Any]]) -> list[str]:
        """llm.tool_call.id is the leading hex of the SHA-256 of the protocol id."""
        wrong = []
        for span in spans:
            if self._attribute(span, "actrail.action.kind") != "llm.tool_call":
                continue
            protocol_id = self._attribute(span, "llm.tool_call.protocol_id")
            short_id = self._attribute(span, "llm.tool_call.id")
            expected = (
                hashlib.sha256(protocol_id.encode()).hexdigest()[: self._SHORT_TOOL_CALL_ID_HEX]
                if protocol_id
                else None
            )
            if short_id != expected:
                wrong.append(f"{short_id} for {protocol_id}")
        return sorted(wrong)

    def _results_without_their_call(self, spans: list[dict[str, Any]]) -> list[str]:
        """A tool result carries the id and tool name of the call it answers."""
        call_keys = ("llm.tool_call.name",)
        calls = {
            self._attribute(span, "llm.tool_call.id"): span
            for span in spans
            if self._attribute(span, "actrail.action.kind") == "llm.tool_call"
        }
        mismatched = []
        for span in spans:
            if self._attribute(span, "actrail.action.kind") != "llm.tool_result":
                continue
            call = calls[self._attribute(span, "llm.tool_call.id")]
            carried = [self._attribute(span, key) for key in call_keys]
            declared = [self._attribute(call, key) for key in call_keys]
            if carried != declared:
                mismatched.append(f"result carries {carried}, call declares {declared}")
        return sorted(mismatched)

    def _tool_ids(
        self,
        spans: list[dict[str, Any]],
        kind: str,
        id_key: str,
    ) -> list[str]:
        ids = []
        for span in spans:
            if self._attribute(span, "actrail.action.kind") != kind:
                continue
            tool_id = self._attribute(span, id_key)
            if not tool_id:
                raise AssertionError(f"{kind} span has no {id_key}")
            ids.append(tool_id)
        return ids

    def _launch(
        self,
        marker: str,
        xiaoo: Path,
        environment: dict[str, str],
        *,
        request_marker: str | None = None,
    ) -> int:
        assert self._environment is not None
        answer_marker = "ready"
        prompt = ""
        if request_marker is not None:
            prompt = f'The opaque request verification marker is "{request_marker}". '
        prompt += (
            "Use the bash tool exactly once to execute the command "
            f"`sleep {CASE_CONSTANTS.command_sleep_seconds}`. Do not run other commands. "
            f'After the tool succeeds reply with exactly "{answer_marker}".'
        )
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
                "--tools",
                "bash",
                "--max-turns",
                str(CASE_CONSTANTS.max_turns),
                "--prompt",
                prompt,
            ],
            timeout_seconds=self._config.launch_timeout_seconds,
            environment=environment,
        )
        if result.returncode != 0:
            raise AssertionError(
                f"actrailctl launch exited with {result.returncode}: "
                f"{result.output[-CASE_CONSTANTS.launch_diagnostic_tail_characters:]}"
            )
        if answer_marker not in result.stdout:
            raise AssertionError(
                f"xiaoO output did not contain answer marker {answer_marker}: "
                f"{result.output[-CASE_CONSTANTS.launch_diagnostic_tail_characters:]}"
            )
        trace_ids = [int(value) for value in self._TRACE_PATTERN.findall(result.output)]
        if len(trace_ids) != 1:
            raise AssertionError(
                f"expected one trace id, found {trace_ids}: {result.output[-CASE_CONSTANTS.launch_diagnostic_tail_characters:]}"
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

        assert self._environment is not None
        database = self._environment.runtime.database_path
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

    def _require_agent_identity_correlation(
        self,
        spans: list[dict[str, Any]],
        otel_trace_id: str,
        agent_binary: Path,
    ) -> str:
        identities = [
            span
            for span in spans
            if self._attribute(span, "actrail.action.kind") == "agent.identity"
        ]
        if len(identities) != 1:
            raise AssertionError(
                f"expected one agent.identity span, found {len(identities)}"
            )
        identity = identities[0]
        if identity.get("traceId") != otel_trace_id:
            raise AssertionError(
                "agent.identity does not share the trace OTLP identity: "
                f"{identity.get('traceId')!r} != {otel_trace_id!r}"
            )
        executable = self._attribute(identity, "process.executable")
        if not executable:
            raise AssertionError("agent.identity has no process.executable")
        if Path(executable).name != agent_binary.name:
            raise AssertionError(
                f"agent.identity executable {executable!r} does not identify "
                f"the launched agent {str(agent_binary)!r}"
            )
        if not self._attribute(identity, "actrail.process.id"):
            raise AssertionError("agent.identity has no process identity")
        self._require_agent_type(identity, agent_binary)
        return executable

    def _require_agent_type(self, identity: dict[str, Any], agent_binary: Path) -> None:
        """The span must name the agent product, not only the binary it ran."""
        agent_type = self._attribute(identity, "actrail.agent.type")
        if agent_type != agent_binary.name:
            raise AssertionError(
                f"agent.identity resolved agent type {agent_type!r}, "
                f"expected {agent_binary.name!r}"
            )
        source = self._attribute(identity, "actrail.agent.type.source")
        if source != "known_product":
            raise AssertionError(
                f"agent type of a known product resolved from {source!r}, "
                "expected 'known_product'"
            )

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

    def _wait_for_marker_spans(
        self,
        marker: str,
        required_kinds: set[str],
    ) -> list[dict[str, Any]]:
        spans: list[dict[str, Any]] = []
        for _ in range(self._config.drain_attempts):
            spans = self._marker_spans(marker)
            kinds = {self._attribute(span, "actrail.action.kind") for span in spans}
            if required_kinds.issubset(kinds):
                return spans
            time.sleep(self._config.drain_interval_seconds)
        raise AssertionError(
            "OTEL/HTTP lifecycle finish did not flush required action kinds; "
            f"required={sorted(required_kinds)}, "
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

    def _require_exported_request_body(
        self,
        spans: list[dict[str, Any]],
        request_marker: str,
    ) -> int:
        request_spans = [
            span
            for span in spans
            if self._attribute(span, "actrail.action.kind") == "llm.request"
        ]
        matching_bodies = 0
        for span in request_spans:
            body_json = self._attribute(span, "llm.request.canonical_body_json")
            if body_json is None:
                continue
            state = self._attribute(
                span,
                "llm.request.canonical_body_export_state",
            )
            if state != "exported":
                raise AssertionError(
                    "OTEL/HTTP request body is present without exported state: "
                    f"{state!r}"
                )
            try:
                body = json.loads(body_json)
            except json.JSONDecodeError as error:
                raise AssertionError(
                    "OTEL/HTTP canonical request body is not valid JSON"
                ) from error
            if self._json_contains_string(body, request_marker):
                matching_bodies += 1
        if matching_bodies < 1:
            raise AssertionError(
                "full OTEL/HTTP export contained no canonical request body "
                f"with user marker {request_marker!r}"
            )
        return matching_bodies

    @classmethod
    def _json_contains_string(cls, value: Any, expected: str) -> bool:
        if isinstance(value, str):
            return expected in value
        if isinstance(value, list):
            return any(cls._json_contains_string(item, expected) for item in value)
        if isinstance(value, dict):
            return any(
                cls._json_contains_string(item, expected)
                for item in value.values()
            )
        return False

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
                f"{result.returncode}: {result.stderr[-CASE_CONSTANTS.command_diagnostic_tail_characters:]}"
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
