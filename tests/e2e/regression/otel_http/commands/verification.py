from __future__ import annotations

import json
from collections import Counter
from pathlib import Path
from typing import Any
from urllib.parse import quote
from urllib.request import ProxyHandler, build_opener

from tests.e2e.common.otel.command_links import CommandLinkVerification

from ..config import OtelHttpConfig
from ..constants import CASE_CONSTANTS


class CommandSpanVerification:
    """Compare real Web command facts with the actual HTTP receiver output."""

    def __init__(self, config: OtelHttpConfig):
        self._config = config
        host = config.web_host
        if ":" in host and not host.startswith("["):
            host = f"[{host}]"
        self._base_url = f"http://{host}:{config.web_port}"
        self._http = build_opener(ProxyHandler({}))

    def verify(
        self,
        trace_id: int,
        mode: str,
        spans: list[dict[str, Any]],
        enabled_kinds: set[str],
    ) -> str:
        commands = self._get(f"/api/traces/{trace_id}/commands")
        local = [
            self._get(
                f"/api/traces/{trace_id}/actions/{quote(action['id'], safe='')}"
            )
            for action in commands["actions"]
            if action.get("kind") == "command.invocation"
        ]
        received = [
            span for span in spans
            if self._attributes(span).get("actrail.action.kind") == "command.invocation"
        ]
        if not local:
            raise AssertionError("real xiaoO produced no local command.invocation")
        unexpected = {
            self._attributes(span).get("actrail.action.kind") for span in spans
        }.difference(enabled_kinds)
        if unexpected:
            raise AssertionError(f"action filtering leaked disabled kinds: {unexpected}")

        counts = Counter(self._attributes(span).get("actrail.action.id") for span in received)
        local_ids = {action["id"] for action in local}
        if set(counts) != local_ids or any(count != 1 for count in counts.values()):
            raise AssertionError(
                "command action IDs must each have exactly one Span: "
                f"local={sorted(local_ids)}, received={dict(counts)}"
            )
        by_id = {self._attributes(span)["actrail.action.id"]: span for span in received}
        CommandLinkVerification(self._get).verify(trace_id, local, by_id)
        sleeping: list[float] = []
        for action in local:
            span = by_id[action["id"]]
            self._require_same_terminal(action, span, mode)
            if Path(action["attributes"].get("process.executable", "")).name == "sleep":
                if (action["status"], action["completeness"]) != ("success", "complete"):
                    raise AssertionError(f"sleep did not finish successfully: {action}")
                if action["attributes"].get("command.exit_code") != "0":
                    raise AssertionError(f"sleep has no successful observed exit: {action}")
                seconds = (
                    int(action["end_time_unix_nanos"]) - int(action["start_time_unix_nanos"])
                ) / 1_000_000_000
                if abs(seconds - CASE_CONSTANTS.command_sleep_seconds) > CASE_CONSTANTS.command_duration_tolerance_seconds:
                    raise AssertionError(
                        f"sleep duration is not about {CASE_CONSTANTS.command_sleep_seconds} seconds: {seconds}"
                    )
                sleeping.append(seconds)
        if not sleeping:
            raise AssertionError("real xiaoO did not execute an observed sleep command")
        return (
            f"{len(local)} command IDs each exported once with matching terminal facts; "
            f"sleep seconds={sleeping}"
        )

    def _require_same_terminal(
        self,
        action: dict[str, Any],
        span: dict[str, Any],
        mode: str,
    ) -> None:
        attributes = self._attributes(span)
        expected = (action["status"], action["completeness"])
        actual = (
            attributes.get("actrail.action.status"),
            attributes.get("actrail.action.completeness"),
        )
        if expected[0] not in {"success", "error", "unknown"} or expected[1] == "partial":
            raise AssertionError(f"local command has no terminal facts: {action}")
        if actual != expected:
            raise AssertionError(
                f"{action['id']} terminal differs: local={expected}, Span={actual}; "
                f"Span start/end={span.get('startTimeUnixNano')}/{span.get('endTimeUnixNano')}"
            )
        status_code = {
            "success": "STATUS_CODE_OK",
            "error": "STATUS_CODE_ERROR",
            "unknown": "STATUS_CODE_UNSET",
        }[action["status"]]
        if span.get("status", {}).get("code") != status_code:
            raise AssertionError(f"{action['id']} OTLP status code differs from Web status")
        start = int(action["start_time_unix_nanos"])
        raw_end = action["end_time_unix_nanos"]
        end = int(raw_end) if raw_end is not None else None
        if int(span["startTimeUnixNano"]) != start or int(span["endTimeUnixNano"]) != (end if end is not None else start):
            raise AssertionError(f"{action['id']} Span start/end differ from Web command details")
        exit_code = action["attributes"].get("command.exit_code")
        if mode == "full" and attributes.get("command.exit_code") != exit_code:
            raise AssertionError(f"{action['id']} Full Span exit code differs from Web details")
        if mode == "metadata-only" and "command.exit_code" in attributes:
            raise AssertionError(f"{action['id']} metadata-only leaked command.exit_code")
        for key in ("command.tool.name", "actrail.action.finalized_on_trace_close"):
            if mode == "full" and attributes.get(key) != action["attributes"].get(key):
                raise AssertionError(f"{action['id']} Full Span {key} differs from Web details")
            if mode == "metadata-only" and key in attributes:
                raise AssertionError(f"{action['id']} metadata-only leaked {key}")

    def _get(self, path: str) -> dict[str, Any]:
        with self._http.open(
            self._base_url + path, timeout=self._config.command_timeout_seconds
        ) as response:
            document = json.load(response)
        if not isinstance(document, dict):
            raise AssertionError(f"Web returned a non-object response for {path}")
        return document

    @staticmethod
    def _attributes(span: dict[str, Any]) -> dict[str, str]:
        return {
            attribute["key"]: str(attribute["value"].get(
                "stringValue", attribute["value"].get("intValue", "")
            ))
            for attribute in span.get("attributes", [])
        }
