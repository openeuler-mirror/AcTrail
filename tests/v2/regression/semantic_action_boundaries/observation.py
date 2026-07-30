from __future__ import annotations

import json
import re
import time
from dataclasses import dataclass
from typing import Any

from tests.v2.common.actrail_runtime import CommandResult

from .environment import SemanticActionBoundariesEnvironment


@dataclass(frozen=True)
class ExportedSpan:
    kind: str
    action_id: str
    process_id: str
    executable: str
    exit_code: str
    status_code: str


class SemanticActionObservation:
    _TRACE_PATTERN = re.compile(r"trace trace-(\d+) entered Active")

    def __init__(
        self,
        environment: SemanticActionBoundariesEnvironment,
    ):
        self._environment = environment

    def require_trace_id(self, launch: CommandResult) -> int:
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

    def wait_for_terminal_trace(self, trace_id: int) -> None:
        last_state = "<missing>"
        for _ in range(self._environment.config.drain_attempts):
            document = self.viewer_json(["traces"])
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

    def wait_for_exported_kinds(
        self,
        marker: str,
        expected: set[str],
    ) -> set[str]:
        actual: set[str] = set()
        for _ in range(self._environment.config.drain_attempts):
            actual = {
                span.kind
                for span in self.extract_exported_spans(marker)
            }
            if actual == expected:
                return actual
            time.sleep(
                self._environment.config.drain_interval_seconds
            )
        raise AssertionError(
            f"online action kinds for {marker} are {sorted(actual)}, "
            f"expected {sorted(expected)}"
        )

    def wait_for_marker_spans(
        self,
        marker: str,
        required_kinds: set[str],
    ) -> list[ExportedSpan]:
        spans: list[ExportedSpan] = []
        for _ in range(self._environment.config.drain_attempts):
            spans = self.extract_exported_spans(marker)
            if required_kinds.issubset(
                {span.kind for span in spans}
            ):
                return spans
            time.sleep(
                self._environment.config.drain_interval_seconds
            )
        raise AssertionError(
            f"online actions for {marker} did not contain "
            f"{sorted(required_kinds)}"
        )

    def extract_exported_spans(
        self,
        marker: str,
    ) -> list[ExportedSpan]:
        path = self._environment.export_path
        if not path.is_file():
            return []
        spans: list[ExportedSpan] = []
        for line in path.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            try:
                document = json.loads(line)
            except json.JSONDecodeError:
                continue
            for resource_spans in document.get(
                "resourceSpans",
                [],
            ):
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
                        attributes = span.get("attributes", [])
                        kind = self._string_attribute(
                            attributes,
                            "actrail.action.kind",
                        )
                        action_id = self._string_attribute(
                            attributes,
                            "actrail.action.id",
                        )
                        process_id = self._string_attribute(
                            attributes,
                            "actrail.process.id",
                        )
                        if not kind or not action_id or not process_id:
                            continue
                        spans.append(
                            ExportedSpan(
                                kind=kind,
                                action_id=action_id,
                                process_id=process_id,
                                executable=(
                                    self._string_attribute(
                                        attributes,
                                        "process.executable",
                                    )
                                    or ""
                                ),
                                exit_code=(
                                    self._string_attribute(
                                        attributes,
                                        "process.exit_code",
                                    )
                                    or ""
                                ),
                                status_code=str(
                                    span.get("status", {}).get("code")
                                    or ""
                                ),
                            )
                        )
        return spans

    def stored_action(
        self,
        trace_id: int,
        action_id: str,
    ) -> dict[str, Any]:
        document = self.viewer_json(
            ["actions", "--trace-id", str(trace_id)]
        )
        actions = document.get("actions")
        if not isinstance(actions, list):
            raise AssertionError(
                "actrailviewer actions returned no actions array"
            )
        matches = [
            action
            for action in actions
            if isinstance(action, dict)
            and action.get("action_id") == action_id
        ]
        if len(matches) != 1:
            raise AssertionError(
                f"stored action {action_id} count is {len(matches)}, "
                "expected 1"
            )
        return matches[0]

    def viewer_json(
        self,
        arguments: list[str],
    ) -> dict[str, Any]:
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
                f"actrailviewer {' '.join(arguments)} returned "
                "invalid JSON"
            ) from error
        if not isinstance(document, dict):
            raise AssertionError(
                "actrailviewer returned non-object JSON"
            )
        return document

    def require_runtime_healthy(self) -> None:
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
                        "semantic observation final state is "
                        f"{plugin.get('state')!r}"
                    )
                if plugin.get("dropped_records") != 0:
                    raise AssertionError(
                        "semantic observation dropped_records="
                        f"{plugin.get('dropped_records')!r}"
                    )
                return
        raise AssertionError(
            "semantic action observation instance is absent from "
            "plugin runtime"
        )

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
