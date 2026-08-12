#!/usr/bin/env python3
"""Shared harness for the local_maas_server record e2e: probe upstream and
server subprocess helpers.
"""

from __future__ import annotations

import json
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any


SERVER_DIR = Path(__file__).resolve().parents[1]
REPO = SERVER_DIR.parents[4]
SERVER_PY = SERVER_DIR / "server.py"

UPSTREAM_DIRECT = {
    "id": "chatcmpl-e2e",
    "object": "chat.completion",
    "created": 0,
    "model": "upstream-model",
    "choices": [
        {
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "I will run the scripted local check.",
                "reasoning_content": (
                    "The test asks for a local tool invocation. "
                    "I should use the advertised tool."
                ),
                "tool_calls": [
                    {
                        "id": "call_e2e",
                        "type": "function",
                        "function": {
                            "name": "run_command",
                            "arguments": '{"cmd": "printf ok"}',
                        },
                    }
                ],
            },
            "finish_reason": "tool_calls",
        }
    ],
    "usage": {
        "prompt_tokens": 10,
        "completion_tokens": 18,
        "total_tokens": 28,
    },
}


def _free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def _json_bytes(payload: dict[str, Any]) -> bytes:
    return json.dumps(
        payload, ensure_ascii=False, separators=(",", ":")
    ).encode("utf-8")


def _sse_lines() -> list[str]:
    chunk = {
        "id": "chatcmpl-e2e",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": "upstream-model",
    }

    def data(fields: dict[str, Any]) -> str:
        return "data: " + json.dumps(
            {**chunk, **fields}, ensure_ascii=False
        )

    return [
        data(
            {
                "choices": [
                    {
                        "index": 0,
                        "delta": {"role": "assistant", "content": ""},
                        "finish_reason": None,
                    }
                ]
            }
        ),
        data(
            {
                "choices": [
                    {
                        "index": 0,
                        "delta": {
                            "reasoning_content": (
                                "The test asks for a local tool invocation. "
                            )
                        },
                        "finish_reason": None,
                    }
                ]
            }
        ),
        data(
            {
                "choices": [
                    {
                        "index": 0,
                        "delta": {
                            "reasoning_content": (
                                "I should use the advertised tool."
                            )
                        },
                        "finish_reason": None,
                    }
                ]
            }
        ),
        data(
            {
                "choices": [
                    {
                        "index": 0,
                        "delta": {
                            "content": "I will run the scripted local check."
                        },
                        "finish_reason": None,
                    }
                ]
            }
        ),
        data(
            {
                "choices": [
                    {
                        "index": 0,
                        "delta": {
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_e2e",
                                    "type": "function",
                                    "function": {
                                        "name": "run_command",
                                        "arguments": "",
                                    },
                                }
                            ]
                        },
                        "finish_reason": None,
                    }
                ]
            }
        ),
        data(
            {
                "choices": [
                    {
                        "index": 0,
                        "delta": {
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "function": {
                                        "arguments": '{"cmd": "printf ok"}'
                                    },
                                }
                            ]
                        },
                        "finish_reason": None,
                    }
                ]
            }
        ),
        data(
            {
                "choices": [
                    {
                        "index": 0,
                        "delta": {},
                        "finish_reason": "tool_calls",
                    }
                ]
            }
        ),
        data(
            {
                "choices": [],
                "usage": {
                    "prompt_tokens": 10,
                    "completion_tokens": 18,
                    "total_tokens": 28,
                },
            }
        ),
        "data: [DONE]",
    ]


class ProbeHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_POST(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        document = json.loads(body.decode("utf-8"))
        with self.server.log_lock:  # type: ignore[attr-defined]
            self.server.log_requests.append(  # type: ignore[attr-defined]
                {
                    "path": self.path,
                    "tools": document.get("tools"),
                    "stream": document.get("stream"),
                    "model": document.get("model"),
                    "authorization": self.headers.get("Authorization"),
                }
            )
        stream = document.get("stream", False)
        if stream:
            payload = "\n\n".join(_sse_lines()) + "\n\n"
            self.close_connection = True
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream; charset=utf-8")
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(payload.encode("utf-8"))
        else:
            payload = _json_bytes(UPSTREAM_DIRECT)
            self.send_response(200)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    def log_message(self, *_args: Any) -> None:
        return


class ProbeUpstream:
    def __init__(self) -> None:
        self.requests: list[dict[str, Any]] = []
        self.lock = threading.Lock()
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), ProbeHandler)
        self.server.log_requests = self.requests
        self.server.log_lock = self.lock
        self.thread = threading.Thread(
            target=self.server.serve_forever, daemon=True
        )

    def start(self) -> str:
        self.thread.start()
        host, port = self.server.server_address[:2]
        return f"http://{host}:{port}"

    def stop(self) -> None:
        self.server.shutdown()
        self.server.server_close()

    def tool_names_by_request(self) -> list[list[str]]:
        names: list[list[str]] = []
        for request in self.requests:
            tools = request["tools"]
            if not isinstance(tools, list):
                raise AssertionError(f"unexpected tools payload: {tools!r}")
            names.append(
                [
                    tool["function"]["name"]
                    for tool in tools
                    if isinstance(tool, dict)
                ]
            )
        return names


class MaaSServerProcess:
    def __init__(self, arguments: list[str], workdir: Path):
        self._process = subprocess.Popen(
            [sys.executable, str(SERVER_PY), *arguments],
            cwd=str(workdir),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        self.port = self._extract_port()

    def _extract_port(self) -> int:
        for argument_index, argument in enumerate(
            self._process.args
        ):
            if argument == "--http-bind-port":
                return int(self._process.args[argument_index + 1])
        raise RuntimeError("http-bind-port is required")

    def wait_ready(self, timeout_seconds: float = 20.0) -> None:
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            if self._process.poll() is not None:
                output = self._process.stdout.read() if self._process.stdout else ""
                raise RuntimeError(
                    f"server exited early:\n{output}"
                )
            try:
                self.request("GET", "/healthz", timeout=1.0)
                return
            except (urllib.error.URLError, ConnectionError, OSError):
                time.sleep(0.1)
        raise RuntimeError("server did not become ready in time")

    def stop(self) -> None:
        if self._process.poll() is None:
            self._process.terminate()
            try:
                self._process.wait(timeout=5.0)
            except subprocess.TimeoutExpired:
                self._process.kill()
                self._process.wait(timeout=5.0)

    def request(
        self,
        method: str,
        path: str,
        *,
        document: dict[str, Any] | None = None,
        api_key: str | None = None,
        timeout: float = 10.0,
    ) -> tuple[int, bytes]:
        url = f"http://127.0.0.1:{self.port}{path}"
        headers: dict[str, str] = {}
        if api_key is not None:
            headers["Authorization"] = f"Bearer {api_key}"
        payload = None
        if document is not None:
            payload = _json_bytes(document)
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(
            url, data=payload, headers=headers, method=method
        )
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                return response.status, response.read()
        except urllib.error.HTTPError as error:
            return error.code, error.read()

