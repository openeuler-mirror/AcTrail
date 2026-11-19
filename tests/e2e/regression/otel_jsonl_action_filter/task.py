from __future__ import annotations

import json
import secrets
import time
from dataclasses import dataclass
from typing import Any

from tests.e2e.common.actrail_runtime import LaunchOptions, ToolAccess
from tests.e2e.common.agents import Agent, AgentFactory
from tests.e2e.common.runner import TestingContextSingleton

from .environment import OtelJsonlActionFilterEnvironment

from .constants import CASE_CONSTANTS
from .commands.verification import CommandDeliveryVerification


@dataclass(frozen=True)
class FilterRound:
    name: str
    label: str
    exporter: str
    enabled_kinds: frozenset[str]
    inject_retryable_failure: bool = CASE_CONSTANTS.inject_retryable_failure


class OtelJsonlActionFilterTask:
    _EXECUTION_CONTEXT_KINDS = frozenset(
        {"process.exec", "file.read", "command.invocation"}
    )
    _LLM_COMPLETE_KINDS = frozenset(
        {"llm.call", "llm.request", "llm.response"}
    )
    _REPRESENTATIVE_KINDS = _EXECUTION_CONTEXT_KINDS | _LLM_COMPLETE_KINDS

    def __init__(
        self,
        environment: OtelJsonlActionFilterEnvironment,
        agent: Agent,
    ):
        self._environment = environment
        self._agent = agent
        self._rounds = self._build_rounds()
        self._commands = CommandDeliveryVerification(environment.config)

    @classmethod
    def _build_rounds(cls) -> tuple[FilterRound, ...]:
        execution_kind = secrets.choice(
            sorted(cls._EXECUTION_CONTEXT_KINDS)
        )
        llm_kind = secrets.choice(sorted(cls._LLM_COMPLETE_KINDS))
        third_kind = secrets.choice(
            sorted(
                cls._REPRESENTATIVE_KINDS.difference(
                    {execution_kind, llm_kind}
                )
            )
        )
        random_three = frozenset(
            {execution_kind, llm_kind, third_kind}
        )
        return (
            FilterRound(
                "execution-context-file",
                "Export execution context to a file",
                "file",
                cls._EXECUTION_CONTEXT_KINDS,
            ),
            FilterRound(
                "llm-complete-json-rpc",
                "Export LLM activity over JSON-RPC",
                "json_rpc_http",
                cls._LLM_COMPLETE_KINDS,
                inject_retryable_failure=True,
            ),
            FilterRound(
                "mixed-random-three-file",
                "Export three selected action kinds to a file",
                "file",
                random_three,
            ),
            FilterRound(
                "representative-combined-json-rpc",
                "Export execution context and LLM activity over JSON-RPC",
                "json_rpc_http",
                cls._REPRESENTATIVE_KINDS,
            ),
        )

    def run(self, case) -> None:
        for round_definition in self._rounds:
            with case.step(round_definition.label) as step:
                actual = self._run_round(round_definition)
                step.message = f"{round_definition.exporter} exported: " + ", ".join(sorted(actual))
        with case.step("Check plugin health"):
            self._require_runtime_healthy()

    def _run_round(self, round_definition: FilterRound) -> set[str]:
        TestingContextSingleton.current().report_progress(
            "Configure the action filter and exporter",
            f"applying {round_definition.name} filter and exporter",
        )
        self._environment.update_selection(
            round_definition.exporter,
            set(round_definition.enabled_kinds),
        )
        failures_before = self._environment.json_rpc_injected_failures
        response_delays_before = (
            self._environment.json_rpc_injected_response_delays
        )
        request_ids_before = len(
            self._environment.json_rpc_request_ids()
        )
        if round_definition.inject_retryable_failure:
            self._environment.fail_next_json_rpc_requests(1)
            self._environment.delay_next_json_rpc_responses(CASE_CONSTANTS.injected_response_delay_seconds)
        marker = (
            f"OTEL_JSONL_FILTER_{round_definition.name}_"
            f"{secrets.token_hex(CASE_CONSTANTS.marker_random_bytes)}"
        )
        TestingContextSingleton.current().report_progress(
            "Run the Agent with the selected action filter",
            f"launching {self._agent.kind} for {round_definition.name}",
        )
        trace_id = self._launch(marker)
        TestingContextSingleton.current().report_progress(
            "Wait for the trace and exported actions",
            f"waiting for {round_definition.name} trace and exported actions",
        )
        self._wait_for_terminal_trace(trace_id)
        self._wait_for_source_actions(trace_id)
        actual = self._wait_for_exported_kinds(
            marker,
            set(round_definition.enabled_kinds),
            round_definition.exporter,
        )
        if "command.invocation" in round_definition.enabled_kinds:
            self._commands.verify(
                trace_id,
                marker,
                lambda: self._exported_documents(round_definition.exporter),
            )
        if (
            round_definition.inject_retryable_failure
            and self._environment.json_rpc_injected_failures
            != failures_before + 1
        ):
            raise AssertionError(
                "JSON-RPC exporter did not retry the injected HTTP failure"
            )
        if round_definition.inject_retryable_failure:
            if (
                self._environment.json_rpc_injected_response_delays
                != response_delays_before + 1
            ):
                raise AssertionError(
                    "JSON-RPC exporter did not reach the delayed response"
                )
            self._require_same_id_retries(request_ids_before)
        return actual

    def _require_same_id_retries(self, start_index: int) -> None:
        observed: list[int] = []
        for _ in range(self._environment.config.drain_attempts):
            observed = self._environment.json_rpc_request_ids()[
                start_index:
            ]
            if len(observed) >= 3:
                if len(set(observed[:3])) != 1:
                    raise AssertionError(
                        "JSON-RPC retries changed request id: "
                        f"{observed[:3]}"
                    )
                return
            time.sleep(
                self._environment.config.drain_interval_seconds
            )
        raise AssertionError(
            "JSON-RPC exporter did not retry both HTTP status and "
            f"response timeout failures; request_ids={observed}"
        )

    def _launch(self, marker: str):
        fixture = self._environment.config.work_dir / "search.txt"
        fixture.write_text("status=ready\nstatus=waiting\n", encoding="utf-8")
        prompt = (
            f"Use the available grep/search tool to find status=ready in {fixture}. "
            "Include line numbers in the search output, then briefly summarize the result."
        )
        options = LaunchOptions(
            name=marker, host_ebpf="auto",
            tool_access=ToolAccess.READ_ONLY,
            max_turns=self._environment.config.agent_max_turns,
        )
        with self._environment.runtime.launch_scope(options):
            with AgentFactory.create(
                self._agent.kind, binary=self._agent.binary, environment=self._agent.environment,
            ).launchWithAcTrailCtl(stream_mode=False) as agent:
                agent.run(prompt)
                agent.join_all(self._environment.config.launch_timeout_seconds)
                result = agent.results[-1]
                if not result.succeeded:
                    raise AssertionError(result.error or "Agent task failed")
                trace_id = agent.launch_result.trace_id
            if agent.returncode != 0:
                raise AssertionError(f"Agent exited with {agent.returncode}")
        return trace_id


    def _wait_for_terminal_trace(self, trace_id: int) -> None:
        last_state = "<missing>"
        for _ in range(self._environment.config.drain_attempts):
            document = self._viewer_json(["traces"])
            traces = document.get("traces")
            if not isinstance(traces, list):
                raise AssertionError(
                    "actrailviewer traces returned no traces array"
                )
            for trace in traces:
                if (
                    isinstance(trace, dict)
                    and trace.get("trace_id_raw") == trace_id
                ):
                    last_state = (
                        f"{trace.get('state')}/{trace.get('health')}"
                    )
                    if last_state in {
                        "Exited/Clean",
                        "Completed/Clean",
                    }:
                        return
            time.sleep(
                self._environment.config.drain_interval_seconds
            )
        raise AssertionError(
            f"trace-{trace_id} did not reach a clean terminal state; "
            f"last={last_state}"
        )

    def _wait_for_source_actions(self, trace_id: int) -> None:
        last_kinds: set[str] = set()
        for _ in range(self._environment.config.drain_attempts):
            document = self._viewer_json(
                ["actions", "--trace-id", str(trace_id)]
            )
            actions = document.get("actions")
            if not isinstance(actions, list):
                raise AssertionError(
                    "actrailviewer actions returned no actions array"
                )
            last_kinds = {
                str(action["kind"])
                for action in actions
                if isinstance(action, dict)
                and isinstance(action.get("kind"), str)
            }
            if self._REPRESENTATIVE_KINDS.issubset(last_kinds):
                return
            time.sleep(
                self._environment.config.drain_interval_seconds
            )
        missing = sorted(
            self._REPRESENTATIVE_KINDS.difference(last_kinds)
        )
        raise AssertionError(
            f"trace-{trace_id} did not produce representative source "
            "action(s): "
            + ", ".join(missing)
        )

    def _wait_for_exported_kinds(
        self,
        marker: str,
        expected: set[str],
        exporter: str,
    ) -> set[str]:
        actual: set[str] = set()
        for _ in range(self._environment.config.drain_attempts):
            actual = self._extract_exported_kinds(marker, exporter)
            if actual == expected:
                return actual
            time.sleep(
                self._environment.config.drain_interval_seconds
            )
        raise AssertionError(
            f"OTEL action kinds for {marker} are {sorted(actual)}, "
            f"expected {sorted(expected)}"
        )

    def _extract_exported_kinds(
        self,
        marker: str,
        exporter: str,
    ) -> set[str]:
        kinds: set[str] = set()
        for document in self._exported_documents(exporter):
            for resource_spans in document.get("resourceSpans", []):
                if not isinstance(resource_spans, dict):
                    continue
                resource = resource_spans.get("resource", {})
                if (
                    self._string_attribute(
                        resource.get("attributes", []),
                        "actrail.trace.display_name",
                    )
                    != marker
                ):
                    continue
                for scope_spans in resource_spans.get(
                    "scopeSpans",
                    [],
                ):
                    if not isinstance(scope_spans, dict):
                        continue
                    for span in scope_spans.get("spans", []):
                        if not isinstance(span, dict):
                            continue
                        kind = self._string_attribute(
                            span.get("attributes", []),
                            "actrail.action.kind",
                        )
                        action_id = self._string_attribute(
                            span.get("attributes", []),
                            "actrail.action.id",
                        )
                        process_id = self._string_attribute(
                            span.get("attributes", []),
                            "actrail.process.id",
                        )
                        if kind and action_id and process_id:
                            kinds.add(kind)
        return kinds

    def _exported_documents(
        self,
        exporter: str,
    ) -> list[dict[str, Any]]:
        if exporter == "json_rpc_http":
            return self._environment.json_rpc_documents()
        if exporter != "file":
            raise AssertionError(f"unknown exporter {exporter}")
        path = self._environment.export_path
        if not path.is_file():
            return []
        documents: list[dict[str, Any]] = []
        for line in path.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            try:
                document = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(document, dict):
                documents.append(document)
        return documents

    @staticmethod
    def _string_attribute(
        attributes: Any,
        key: str,
    ) -> str | None:
        if not isinstance(attributes, list):
            return None
        for attribute in attributes:
            if (
                not isinstance(attribute, dict)
                or attribute.get("key") != key
            ):
                continue
            value = attribute.get("value")
            if not isinstance(value, dict):
                return None
            if isinstance(value.get("stringValue"), str):
                return value["stringValue"]
            if isinstance(value.get("intValue"), (str, int)):
                return str(value["intValue"])
        return None

    def _viewer_json(self, arguments: list[str]) -> dict[str, Any]:
        result = self._environment.runtime.run(
            [
                self._environment.runtime.actrailviewer,
                "--config",
                self._environment.config.operator_config,
                "--output-format",
                "json",
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
            raise AssertionError(
                f"actrailviewer {' '.join(arguments)} returned invalid JSON"
            ) from error
        if not isinstance(document, dict):
            raise AssertionError(
                "actrailviewer returned non-object JSON"
            )
        return document

    def _require_runtime_healthy(self) -> None:
        document = self._environment.api.runtime()
        plugins = document.get("plugins")
        if not isinstance(plugins, list):
            raise AssertionError(
                "plugin runtime response has no plugins array"
            )
        for plugin in plugins:
            if (
                isinstance(plugin, dict)
                and plugin.get("instance_id")
                == self._environment.config.plugin_instance
            ):
                if plugin.get("state") != "active":
                    raise AssertionError(
                        "otel-jsonl final state is "
                        f"{plugin.get('state')!r}"
                    )
                if plugin.get("dropped_records") != 0:
                    raise AssertionError(
                        "otel-jsonl dropped_records="
                        f"{plugin.get('dropped_records')!r}"
                    )
                return
        raise AssertionError(
            "otel-jsonl instance is absent from plugin runtime"
        )
