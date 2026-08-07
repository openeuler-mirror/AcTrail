from __future__ import annotations

import json
import re
import shutil
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any

STDIO_CAPTURE_ABI_MAX_BYTES = 4095

_MCP_PROBE_SERVER = (
    Path(__file__).resolve().parents[1]
    / "test_suite_tools"
    / "mcp"
    / "mcp_probe_server.py"
)


@dataclass(frozen=True)
class McpProbeSpec:
    server_name: str
    tool_name: str
    marker: str
    event_log: Path
    tool_description_padding_bytes: int = 0

    _NAME_PATTERN = re.compile(r"^[A-Za-z][A-Za-z0-9_-]*$")

    def __post_init__(self) -> None:
        for label, value in (
            ("server_name", self.server_name),
            ("tool_name", self.tool_name),
        ):
            if self._NAME_PATTERN.fullmatch(value) is None:
                raise ValueError(f"invalid MCP probe {label}: {value!r}")
        if not self.marker or any(character.isspace() for character in self.marker):
            raise ValueError(
                "MCP probe marker must be nonempty and contain no whitespace"
            )
        if self.tool_description_padding_bytes < 0:
            raise ValueError(
                "MCP probe tool description padding bytes must be nonnegative"
            )

    @property
    def tool_id(self) -> str:
        return f"mcp__{self.server_name}__{self.tool_name}"


class McpProbeWorkspace:
    _ROOT_MARKER = ".actrail-mcp-test-root"

    def __init__(
        self,
        repo: Path,
        artifact_root: Path,
        case_name: str,
    ) -> None:
        self._repo = repo.resolve()
        self._probe_script = _MCP_PROBE_SERVER
        if not self._probe_script.is_file():
            raise RuntimeError(
                f"repository-owned MCP probe server is missing: {self._probe_script}"
            )
        root = (
            artifact_root
            if artifact_root.is_absolute()
            else self._repo / artifact_root
        )
        root.mkdir(parents=True, exist_ok=True)
        self._artifact_root = root.resolve()
        self.path = Path(
            tempfile.mkdtemp(
                prefix=f"{case_name}-",
                dir=self._artifact_root,
            )
        ).resolve()
        (self.path / self._ROOT_MARKER).write_text(
            "owned by the AcTrail MCP v2 regression test\n",
            encoding="utf-8",
        )
        self._closed = False

    def spec(
        self,
        *,
        server_name: str,
        tool_name: str,
        marker: str,
        tool_description_padding_bytes: int = 0,
    ) -> McpProbeSpec:
        return McpProbeSpec(
            server_name=server_name,
            tool_name=tool_name,
            marker=marker,
            event_log=self.path / f"{server_name}.events.jsonl",
            tool_description_padding_bytes=tool_description_padding_bytes,
        )

    def stdio_command(self, spec: McpProbeSpec) -> tuple[str, list[str]]:
        return str(Path(sys.executable).resolve()), self.server_arguments(spec)

    def server_arguments(self, spec: McpProbeSpec) -> list[str]:
        return [
            str(self._probe_script),
            "--server-name",
            spec.server_name,
            "--tool-name",
            spec.tool_name,
            "--marker",
            spec.marker,
            "--event-log",
            str(spec.event_log),
            "--tool-description-padding-bytes",
            str(spec.tool_description_padding_bytes),
        ]

    def write_claude_config(self, spec: McpProbeSpec) -> Path:
        command, arguments = self.stdio_command(spec)
        config_path = self.path / "claude-mcp.json"
        document = {
            "mcpServers": {
                spec.server_name: {
                    "command": command,
                    "args": arguments,
                }
            }
        }
        config_path.write_text(
            json.dumps(document, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        return config_path

    def require_execution(self, spec: McpProbeSpec) -> str:
        events = self._read_events(spec.event_log)
        executions = [
            event for event in events if event.get("event") == "tool_execution"
        ]
        if len(executions) != 1:
            raise AssertionError(
                f"{spec.tool_id} must execute exactly once; "
                f"found {len(executions)} execution(s) in {spec.event_log}; "
                f"events={self._event_summary(events)}"
            )
        execution = executions[0]
        expected = {
            "server": spec.server_name,
            "tool": spec.tool_name,
            "marker": spec.marker,
            "arguments": {"marker": spec.marker},
        }
        actual = {key: execution.get(key) for key in expected}
        if actual != expected:
            raise AssertionError(
                f"{spec.tool_id} execution evidence mismatch: "
                f"expected={expected} actual={actual}"
            )
        request_id = execution.get("request_id")
        if request_id is None:
            raise AssertionError(f"{spec.tool_id} execution has no JSON-RPC request id")
        requests = [
            event
            for event in events
            if event.get("event") == "message"
            and event.get("direction") == "client_to_server"
            and isinstance(event.get("message"), dict)
            and event["message"].get("method") == "tools/call"
        ]
        if len(requests) != 1:
            raise AssertionError(
                f"{spec.tool_id} must have exactly one tools/call request; "
                f"found {len(requests)}"
            )
        request = requests[0]["message"]
        if request != {
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "tools/call",
            "params": {
                "name": spec.tool_name,
                "arguments": {"marker": spec.marker},
            },
        }:
            raise AssertionError(
                f"{spec.tool_id} tools/call request mismatch: {request}"
            )
        responses = [
            event["message"]
            for event in events
            if event.get("event") == "message"
            and event.get("direction") == "server_to_client"
            and isinstance(event.get("message"), dict)
            and event["message"].get("id") == request_id
        ]
        expected_response = {
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "content": [{"type": "text", "text": spec.marker}],
                "structuredContent": {"marker": spec.marker},
                "isError": False,
            },
        }
        if responses != [expected_response]:
            raise AssertionError(
                f"{spec.tool_id} response evidence mismatch: {responses}"
            )
        self._require_tool_listing_size(spec, events)
        return f"{spec.tool_id} executed once with request id {request_id}"

    def tool_listing_completed(self, spec: McpProbeSpec) -> bool:
        if not spec.event_log.is_file():
            return False
        events = self._read_events(spec.event_log)
        request_ids = {
            event["message"].get("id")
            for event in events
            if event.get("event") == "message"
            and event.get("direction") == "client_to_server"
            and isinstance(event.get("message"), dict)
            and event["message"].get("method") == "tools/list"
        }
        for event in events:
            message = event.get("message")
            if (
                event.get("event") != "message"
                or event.get("direction") != "server_to_client"
                or not isinstance(message, dict)
                or message.get("id") not in request_ids
            ):
                continue
            result = message.get("result")
            tools = result.get("tools") if isinstance(result, dict) else None
            if isinstance(tools, list) and any(
                isinstance(tool, dict) and tool.get("name") == spec.tool_name
                for tool in tools
            ):
                return True
        return False

    def close(self) -> None:
        if self._closed:
            return
        if (
            self.path.parent != self._artifact_root
            or not self.path.name.startswith("probe_")
            or not (self.path / self._ROOT_MARKER).is_file()
        ):
            raise RuntimeError(
                f"refusing to clean unverified MCP test workspace: {self.path}"
            )
        shutil.rmtree(self.path)
        self._closed = True

    @staticmethod
    def _require_tool_listing_size(
        spec: McpProbeSpec,
        events: list[dict[str, Any]],
    ) -> None:
        if spec.tool_description_padding_bytes == 0:
            return
        request_ids = {
            event["message"].get("id")
            for event in events
            if event.get("event") == "message"
            and event.get("direction") == "client_to_server"
            and isinstance(event.get("message"), dict)
            and event["message"].get("method") == "tools/list"
        }
        sizes = [
            len(
                json.dumps(
                    event["message"],
                    separators=(",", ":"),
                ).encode("utf-8")
            )
            for event in events
            if event.get("event") == "message"
            and event.get("direction") == "server_to_client"
            and isinstance(event.get("message"), dict)
            and event["message"].get("id") in request_ids
        ]
        if not sizes or max(sizes) <= STDIO_CAPTURE_ABI_MAX_BYTES:
            raise AssertionError(
                f"{spec.tool_id} tools/list response did not exceed the stdio "
                f"capture ABI boundary; sizes={sizes} "
                f"boundary={STDIO_CAPTURE_ABI_MAX_BYTES}"
            )

    @staticmethod
    def _read_events(path: Path) -> list[dict[str, Any]]:
        if not path.is_file():
            raise AssertionError(f"MCP probe event log is missing: {path}")
        events: list[dict[str, Any]] = []
        for line_number, raw in enumerate(
            path.read_text(encoding="utf-8").splitlines(),
            start=1,
        ):
            try:
                value = json.loads(raw)
            except json.JSONDecodeError as error:
                raise AssertionError(
                    f"{path}:{line_number} is not valid JSONL"
                ) from error
            if not isinstance(value, dict):
                raise AssertionError(
                    f"{path}:{line_number} must contain a JSON object"
                )
            events.append(value)
        return events

    @staticmethod
    def _event_summary(events: list[dict[str, Any]]) -> list[dict[str, Any]]:
        summary: list[dict[str, Any]] = []
        for event in events:
            message = event.get("message")
            summary.append(
                {
                    "event": event.get("event"),
                    "direction": event.get("direction"),
                    "method": (
                        message.get("method")
                        if isinstance(message, dict)
                        else None
                    ),
                    "request_id": event.get(
                        "request_id",
                        message.get("id") if isinstance(message, dict) else None,
                    ),
                    "tool": event.get("tool"),
                }
            )
        return summary
