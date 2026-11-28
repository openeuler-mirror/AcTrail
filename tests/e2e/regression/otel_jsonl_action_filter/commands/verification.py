from __future__ import annotations

import json
import time
from collections import Counter
from collections.abc import Callable
from typing import Any
from urllib.parse import quote
from urllib.request import ProxyHandler, build_opener

from tests.e2e.common.otel.command_links import CommandLinkVerification

from ..config import OtelJsonlActionFilterConfig
from ..constants import CASE_CONSTANTS


class CommandDeliveryVerification:
    """Compare completed real commands with the file or JSON-RPC delivery."""

    def __init__(self, config: OtelJsonlActionFilterConfig):
        self._config = config
        host = config.web_host
        if ":" in host and not host.startswith("["):
            host = f"[{host}]"
        self._base_url = f"http://{host}:{config.web_port}"
        self._http = build_opener(ProxyHandler({}))

    def verify(
        self,
        trace_id: int,
        marker: str,
        documents: Callable[[], list[dict[str, Any]]],
    ) -> None:
        commands = self._get(f"/api/traces/{trace_id}/commands")
        local = [
            self._get(f"/api/traces/{trace_id}/actions/{quote(action['id'], safe='')}")
            for action in commands["actions"]
            if action.get("kind") == "command.invocation"
        ]
        if not local:
            raise AssertionError("real agent trace produced no local commands")
        expected = {action["id"] for action in local}
        complete_since: float | None = None
        received: list[dict[str, Any]] = []
        for _ in range(self._config.drain_attempts):
            received = self._commands(marker, documents())
            counts = Counter(self._attributes(span)["actrail.action.id"] for span in received)
            if set(counts).difference(expected) or any(count != 1 for count in counts.values()):
                raise AssertionError(f"command IDs must each be exported once: {dict(counts)}")
            if set(counts) == expected:
                if complete_since is None:
                    complete_since = time.monotonic()
                if time.monotonic() - complete_since >= CASE_CONSTANTS.command_delivery_settle_seconds:
                    break
            else:
                complete_since = None
            time.sleep(self._config.drain_interval_seconds)
        else:
            raise AssertionError(
                f"terminal command delivery did not finish: local={sorted(expected)}, "
                f"received={[self._attributes(span).get('actrail.action.id') for span in received]}"
            )
        by_id = {self._attributes(span)["actrail.action.id"]: span for span in received}
        for action in local:
            span = by_id[action["id"]]
            attributes = self._attributes(span)
            terminal = (action["status"], action["completeness"])
            if terminal[0] not in {"success", "error", "unknown"} or terminal[1] == "partial":
                raise AssertionError(f"local command has no terminal facts: {action}")
            if terminal != (
                attributes.get("actrail.action.status"),
                attributes.get("actrail.action.completeness"),
            ):
                raise AssertionError(f"{action['id']} terminal delivery differs from Web")
            start = int(action["start_time_unix_nanos"])
            raw_end = action["end_time_unix_nanos"]
            end = int(raw_end) if raw_end is not None else start
            if int(span["startTimeUnixNano"]) != start or int(span["endTimeUnixNano"]) != end:
                raise AssertionError(f"{action['id']} Span times differ from Web")
            if attributes.get("command.exit_code") != action["attributes"].get("command.exit_code"):
                raise AssertionError(f"{action['id']} exit code differs from Web")
            for key in ("command.tool.name", "actrail.action.finalized_on_trace_close"):
                if attributes.get(key) != action["attributes"].get(key):
                    raise AssertionError(f"{action['id']} {key} differs from Web")
        CommandLinkVerification(self._get).verify(
            trace_id, local, by_id
        )

    def _get(self, path: str) -> dict[str, Any]:
        with self._http.open(
            self._base_url + path, timeout=self._config.command_timeout_seconds
        ) as response:
            return json.load(response)

    def _commands(self, marker: str, documents: list[dict[str, Any]]) -> list[dict[str, Any]]:
        commands = []
        for document in documents:
            for resource in document.get("resourceSpans", []):
                resource_attributes = self._attributes(resource.get("resource", {}))
                if resource_attributes.get("actrail.trace.display_name") != marker:
                    continue
                for scope in resource.get("scopeSpans", []):
                    for span in scope.get("spans", []):
                        if self._attributes(span).get("actrail.action.kind") == "command.invocation":
                            commands.append(span)
        return commands

    @staticmethod
    def _attributes(value: dict[str, Any]) -> dict[str, str]:
        return {
            attribute["key"]: str(attribute["value"].get(
                "stringValue", attribute["value"].get("intValue", "")
            ))
            for attribute in value.get("attributes", [])
        }
