#!/usr/bin/env python3
"""Serve real OpenAI-compatible tool-call turns for the activity alert E2E."""

from __future__ import annotations

import argparse
import json
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bind-host", default="0.0.0.0")
    parser.add_argument("--bind-port", type=int, default=0)
    parser.add_argument("--response-marker", required=True)
    parser.add_argument("--sleep-seconds", type=float, default=2.0)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.bind_port < 0:
        raise RuntimeError("--bind-port must be non-negative")
    if args.sleep_seconds <= 0:
        raise RuntimeError("--sleep-seconds must be positive")
    server = ThreadingHTTPServer(
        (args.bind_host, args.bind_port),
        make_handler(args.response_marker, args.sleep_seconds),
    )
    host, port = server.server_address
    print(f"provider_base_url=http://{host}:{port}", flush=True)
    try:
        server.serve_forever()
    finally:
        server.server_close()
    return 0


def make_handler(response_marker: str, sleep_seconds: float):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def do_POST(self) -> None:
            try:
                body = read_body(self)
                request = parse_request(body)
                if has_tool_result(request):
                    chunks = final_chunks(request, response_marker)
                    turn = "final"
                else:
                    chunks = tool_call_chunks(request, sleep_seconds)
                    turn = "tool"
                write_stream(self, chunks)
                print(
                    f"activity_provider turn={turn} path={self.path} "
                    f"request_bytes={len(body)} response_events={len(chunks)}",
                    flush=True,
                )
            except Exception as error:
                print(f"activity_provider_error={error}", file=sys.stderr, flush=True)
                if not self.wfile.closed:
                    self.send_error(500, "activity provider failure")

        def log_message(self, *_args) -> None:
            return

    return Handler


def read_body(handler: BaseHTTPRequestHandler) -> bytes:
    length = int(handler.headers.get("Content-Length", "0"))
    if length <= 0:
        raise RuntimeError("request has no Content-Length")
    return handler.rfile.read(length)


def parse_request(body: bytes) -> dict:
    try:
        request = json.loads(body.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise RuntimeError(f"invalid request JSON: {error}") from error
    if not isinstance(request, dict):
        raise RuntimeError("request JSON must be an object")
    if not isinstance(request.get("messages"), list):
        raise RuntimeError("request has no messages array")
    return request


def has_tool_result(request: dict) -> bool:
    for message in request["messages"]:
        if not isinstance(message, dict):
            continue
        if message.get("role") == "tool":
            return True
        content = message.get("content")
        if isinstance(content, list) and any(
            isinstance(block, dict)
            and block.get("type") in {"tool_result", "tool-result"}
            for block in content
        ):
            return True
    return False


def tool_call_chunks(request: dict, sleep_seconds: float) -> list[dict]:
    name, arguments = bash_tool_call(request, sleep_seconds)
    model = str(request.get("model") or "actrail-activity-e2e")
    call_id = "call_actrail_activity_sleep"
    return [
        chunk(
            model,
            {
                "role": "assistant",
                "tool_calls": [
                    {
                        "index": 0,
                        "id": call_id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": json.dumps(
                                arguments, ensure_ascii=False, separators=(",", ":")
                            ),
                        },
                    }
                ],
            },
            None,
        ),
        chunk(model, {}, "tool_calls"),
    ]


def bash_tool_call(request: dict, sleep_seconds: float) -> tuple[str, dict]:
    tools = request.get("tools")
    if not isinstance(tools, list):
        raise RuntimeError("real agent request did not advertise tools")
    candidates: list[tuple[str, dict]] = []
    for tool in tools:
        if not isinstance(tool, dict):
            continue
        function = tool.get("function")
        if not isinstance(function, dict):
            continue
        name = function.get("name")
        parameters = function.get("parameters")
        if isinstance(name, str) and isinstance(parameters, dict):
            candidates.append((name, parameters))
    for name, parameters in candidates:
        if "bash" not in name.lower():
            continue
        properties = parameters.get("properties")
        if not isinstance(properties, dict):
            raise RuntimeError(f"bash tool {name} has no properties schema")
        command_key = next(
            (key for key in ("command", "cmd") if key in properties),
            None,
        )
        if command_key is None:
            raise RuntimeError(
                f"bash tool {name} exposes no command/cmd input: {sorted(properties)}"
            )
        return name, {command_key: f"sleep {sleep_seconds:g}"}
    raise RuntimeError(
        "real agent did not advertise a bash tool: "
        + ",".join(name for name, _ in candidates)
    )


def final_chunks(request: dict, response_marker: str) -> list[dict]:
    model = str(request.get("model") or "actrail-activity-e2e")
    parts = [response_marker[index : index + 8] for index in range(0, len(response_marker), 8)]
    chunks = [chunk(model, {"content": part}, None) for part in parts]
    chunks.append(chunk(model, {}, "stop", include_usage=True))
    return chunks


def chunk(
    model: str,
    delta: dict,
    finish_reason: str | None,
    include_usage: bool = False,
) -> dict:
    value = {
        "id": "chatcmpl-actrail-activity",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": model,
        "choices": [
            {
                "index": 0,
                "delta": delta,
                "finish_reason": finish_reason,
            }
        ],
    }
    if include_usage:
        value["usage"] = {
            "prompt_tokens": 1,
            "completion_tokens": 1,
            "total_tokens": 2,
        }
    return value


def write_stream(handler: BaseHTTPRequestHandler, chunks: list[dict]) -> None:
    handler.send_response(200, "OK")
    handler.send_header("Content-Type", "text/event-stream")
    handler.send_header("Cache-Control", "no-cache")
    handler.send_header("Transfer-Encoding", "chunked")
    handler.send_header("Connection", "close")
    handler.end_headers()
    for chunk_value in chunks:
        payload = (
            "data: "
            + json.dumps(chunk_value, ensure_ascii=False, separators=(",", ":"))
            + "\n\n"
        ).encode("utf-8")
        write_chunk(handler, payload)
        handler.wfile.flush()
        time.sleep(0.02)
    write_chunk(handler, b"data: [DONE]\n\n")
    handler.wfile.write(b"0\r\n\r\n")
    handler.wfile.flush()
    handler.close_connection = True


def write_chunk(handler: BaseHTTPRequestHandler, payload: bytes) -> None:
    handler.wfile.write(f"{len(payload):X}\r\n".encode("ascii"))
    handler.wfile.write(payload)
    handler.wfile.write(b"\r\n")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"activity tool provider failed: {error}", file=sys.stderr)
        raise SystemExit(1)
