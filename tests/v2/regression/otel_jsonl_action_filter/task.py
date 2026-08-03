from __future__ import annotations

import json
import re
import secrets
import time
from dataclasses import dataclass
from typing import Any

from tests.v2.common.actrail_runtime import CommandResult
from tests.v2.common.agent_selection import AgentSelection
from tests.v2.common.test_case import TestResult, TestStatus
from tests.v2.common.testing_context import TestingContextSingleton

from .environment import OtelJsonlActionFilterEnvironment


@dataclass(frozen=True)
class FilterRound:
    name: str
    exporter: str
    enabled_kinds: frozenset[str]
    inject_retryable_failure: bool = False


class OtelJsonlActionFilterTask:
    _TRACE_PATTERN = re.compile(r"trace trace-(\d+) entered Active")
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
        agent: AgentSelection,
        test_context: TestingContextSingleton,
    ):
        self._environment = environment
        self._agent = agent
        self._test_context = test_context
        self._rounds = self._build_rounds()

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
                "file",
                cls._EXECUTION_CONTEXT_KINDS,
            ),
            FilterRound(
                "llm-complete-json-rpc",
                "json_rpc_http",
                cls._LLM_COMPLETE_KINDS,
                inject_retryable_failure=True,
            ),
            FilterRound("mixed-random-three-file", "file", random_three),
            FilterRound(
                "representative-combined-json-rpc",
                "json_rpc_http",
                cls._REPRESENTATIVE_KINDS,
            ),
        )

    def run(self) -> dict[str, TestResult]:
        results: dict[str, TestResult] = {}
        for index, round_definition in enumerate(self._rounds, start=1):
            self._test_context.report_progress(
                "filter_round",
                f"{round_definition.name} via {round_definition.exporter} "
                f"({index}/{len(self._rounds)})",
            )
            actual = self._run_round(round_definition)
            results[round_definition.name] = TestResult(
                TestStatus.PASSED,
                f"{round_definition.exporter} exported action kinds: "
                + ", ".join(sorted(actual)),
            )
        self._test_context.report_progress(
            "runtime_health",
            "checking otel-jsonl runtime health",
        )
        self._require_runtime_healthy()
        results["plugin_runtime"] = TestResult(
            TestStatus.PASSED,
            "file and JSON-RPC exporters remain active with dropped_records=0",
        )
        return results

    def _run_round(self, round_definition: FilterRound) -> set[str]:
        self._test_context.report_progress(
            "filter_config",
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
            self._environment.delay_next_json_rpc_responses(0.75)
        marker = (
            f"OTEL_JSONL_FILTER_{round_definition.name}_"
            f"{secrets.token_hex(6)}"
        )
        self._test_context.report_progress(
            "filter_launch",
            f"launching {self._agent.kind} for {round_definition.name}",
        )
        launch = self._launch(marker)
        if launch.returncode != 0:
            raise AssertionError(
                f"{round_definition.name}: actrailctl launch exited with "
                f"{launch.returncode}\n{launch.output[-4000:]}"
            )
        if marker not in launch.output:
            raise AssertionError(
                f"{round_definition.name}: {self._agent.kind} output "
                f"does not contain marker {marker}"
            )
        self._test_context.report_progress(
            "filter_observe",
            f"waiting for {round_definition.name} trace and exported actions",
        )
        trace_id = self._require_trace_id(launch)
        self._wait_for_terminal_trace(trace_id)
        self._wait_for_source_actions(trace_id)
        actual = self._wait_for_exported_kinds(
            marker,
            set(round_definition.enabled_kinds),
            round_definition.exporter,
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

    def _launch(self, marker: str) -> CommandResult:
        prompt = (
            f'Reply with exactly "{marker}" and nothing else. Do not use tools.'
        )
        agent_command = self._agent.command(prompt)
        return self._environment.runtime.run(
            [
                self._environment.runtime.actrailctl,
                "--config",
                self._environment.config.operator_config,
                "launch",
                "--name",
                marker,
                "--",
                "bash",
                "-lc",
                'cat /etc/hostname >/dev/null; exec "$@"',
                "actrail-otel-jsonl-filter",
                *agent_command,
            ],
            timeout_seconds=self._environment.config.launch_timeout_seconds,
            environment=self._agent.environment,
        )

    def _require_trace_id(self, launch: CommandResult) -> int:
        trace_ids = [
            int(value)
            for value in self._TRACE_PATTERN.findall(launch.output)
        ]
        if len(trace_ids) != 1:
            raise AssertionError(
                f"expected one trace id, found {trace_ids}: "
                f"{launch.output[-4000:]}"
            )
        return trace_ids[0]

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
                f"{result.returncode}: {result.stderr[-2000:]}"
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
