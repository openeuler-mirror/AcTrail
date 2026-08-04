#!/usr/bin/env python3
"""Repository-owned stdio MCP probe server for real-agent regression tests."""

from __future__ import annotations

import argparse
import json
import os
import sys
import threading
import time
from pathlib import Path
from typing import Any


class EventRecorder:
    def __init__(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        self._stream = path.open("a", encoding="utf-8")
        self._lock = threading.Lock()

    def message(self, direction: str, message: dict[str, Any]) -> None:
        self._write(
            {
                "event": "message",
                "direction": direction,
                "message": self._without_meta(message),
                "pid": os.getpid(),
                "time_unix_ms": int(time.time() * 1000),
            }
        )

    def tool_execution(
        self,
        server_name: str,
        tool_name: str,
        marker: str,
        arguments: dict[str, Any],
        request_id: Any,
    ) -> None:
        self._write(
            {
                "event": "tool_execution",
                "server": server_name,
                "tool": tool_name,
                "marker": marker,
                "arguments": arguments,
                "request_id": request_id,
                "pid": os.getpid(),
                "time_unix_ms": int(time.time() * 1000),
            }
        )

    def close(self) -> None:
        with self._lock:
            self._stream.close()

    def _write(self, value: dict[str, Any]) -> None:
        encoded = json.dumps(value, separators=(",", ":"), sort_keys=True)
        with self._lock:
            self._stream.write(encoded + "\n")
            self._stream.flush()

    @classmethod
    def _without_meta(cls, value: Any) -> Any:
        if isinstance(value, dict):
            return {
                key: cls._without_meta(child)
                for key, child in value.items()
                if key != "_meta"
            }
        if isinstance(value, list):
            return [cls._without_meta(child) for child in value]
        return value


class McpApplication:
    def __init__(
        self,
        server_name: str,
        tool_name: str,
        marker: str,
        tool_description_padding_bytes: int,
        recorder: EventRecorder,
    ) -> None:
        if tool_description_padding_bytes < 0:
            raise ValueError("tool description padding bytes must be nonnegative")
        self.server_name = server_name
        self.tool_name = tool_name
        self.marker = marker
        self._tool_description_padding = "X" * tool_description_padding_bytes
        self._recorder = recorder

    def handle(self, message: Any) -> dict[str, Any] | None:
        if not isinstance(message, dict) or message.get("jsonrpc") != "2.0":
            return self._error(
                message.get("id") if isinstance(message, dict) else None,
                -32600,
                "invalid JSON-RPC request",
            )
        method = message.get("method")
        request_id = message.get("id")
        if not isinstance(method, str):
            return self._error(request_id, -32600, "request method is required")
        if request_id is None:
            return None
        if method == "initialize":
            return self._result(request_id, self._initialize_result(message))
        if method == "ping":
            return self._result(request_id, {})
        if method == "tools/list":
            return self._result(request_id, {"tools": [self._tool_descriptor()]})
        if method == "tools/call":
            return self._call_tool(message)
        return self._error(request_id, -32601, f"method not found: {method}")

    def _initialize_result(self, message: dict[str, Any]) -> dict[str, Any]:
        params = message.get("params")
        requested_version = "2025-06-18"
        if isinstance(params, dict) and isinstance(params.get("protocolVersion"), str):
            requested_version = params["protocolVersion"]
        return {
            "protocolVersion": requested_version,
            "capabilities": {"tools": {"listChanged": False}},
            "serverInfo": {"name": self.server_name, "version": "1.0.0"},
            "instructions": (
                f"Call {self.tool_name} exactly once with marker={self.marker} "
                "when the user requests the AcTrail MCP regression probe."
            ),
        }

    def _tool_descriptor(self) -> dict[str, Any]:
        return {
            "name": self.tool_name,
            "description": (
                "Echo the required marker and record actual tool execution."
                + self._tool_description_padding
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "marker": {
                        "type": "string",
                        "const": self.marker,
                        "description": (
                            "Exact marker required by this probe invocation."
                        ),
                    }
                },
                "required": ["marker"],
                "additionalProperties": False,
            },
            "annotations": {
                "readOnlyHint": True,
                "destructiveHint": False,
                "idempotentHint": True,
                "openWorldHint": False,
            },
        }

    def _call_tool(self, message: dict[str, Any]) -> dict[str, Any]:
        request_id = message["id"]
        params = message.get("params")
        if not isinstance(params, dict) or params.get("name") != self.tool_name:
            return self._error(request_id, -32602, "unexpected MCP tool name")
        arguments = params.get("arguments")
        if not isinstance(arguments, dict) or arguments != {"marker": self.marker}:
            return self._error(request_id, -32602, "unexpected MCP tool arguments")
        self._recorder.tool_execution(
            self.server_name,
            self.tool_name,
            self.marker,
            arguments,
            request_id,
        )
        return self._result(
            request_id,
            {
                "content": [{"type": "text", "text": self.marker}],
                "structuredContent": {"marker": self.marker},
                "isError": False,
            },
        )

    @staticmethod
    def _result(request_id: Any, result: dict[str, Any]) -> dict[str, Any]:
        return {"jsonrpc": "2.0", "id": request_id, "result": result}

    @staticmethod
    def _error(request_id: Any, code: int, message: str) -> dict[str, Any]:
        return {
            "jsonrpc": "2.0",
            "id": request_id,
            "error": {"code": code, "message": message},
        }


class StdioMcpServer:
    def __init__(self, application: McpApplication, recorder: EventRecorder) -> None:
        self._application = application
        self._recorder = recorder

    def run(self) -> int:
        for raw in sys.stdin.buffer:
            try:
                message = json.loads(raw)
            except json.JSONDecodeError as error:
                raise RuntimeError("stdin contained invalid JSON") from error
            if not isinstance(message, dict):
                raise RuntimeError("stdin MCP messages must be JSON objects")
            self._recorder.message("client_to_server", message)
            response = self._application.handle(message)
            if response is None:
                continue
            sys.stdout.write(json.dumps(response, separators=(",", ":")) + "\n")
            sys.stdout.flush()
            self._recorder.message("server_to_client", response)
        return 0


class ProbeServerCommand:
    def __init__(self, arguments: argparse.Namespace) -> None:
        self._recorder = EventRecorder(Path(arguments.event_log))
        self._application = McpApplication(
            arguments.server_name,
            arguments.tool_name,
            arguments.marker,
            arguments.tool_description_padding_bytes,
            self._recorder,
        )

    def run(self) -> int:
        try:
            return StdioMcpServer(self._application, self._recorder).run()
        finally:
            self._recorder.close()

    @classmethod
    def parse(cls) -> "ProbeServerCommand":
        parser = argparse.ArgumentParser(description=__doc__)
        parser.add_argument("--server-name", required=True)
        parser.add_argument("--tool-name", required=True)
        parser.add_argument("--marker", required=True)
        parser.add_argument("--event-log", required=True)
        parser.add_argument(
            "--tool-description-padding-bytes",
            type=int,
            default=0,
        )
        return cls(parser.parse_args())


if __name__ == "__main__":
    raise SystemExit(ProbeServerCommand.parse().run())
