#!/usr/bin/env python3
"""Plain HTTP OpenAI-compatible reverse provider proxy for agent tests."""

from __future__ import annotations

import argparse
import http.client
import json
import os
import sys
import time
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit


HOP_BY_HOP_HEADERS = {
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
}

DEFAULT_BIND_HOST = "127.0.0.1"
DEFAULT_BIND_PORT = 18098
DEFAULT_UPSTREAM_BASE_URL = "https://api.deepseek.com"
DEFAULT_UPSTREAM_API_KEY_ENV = "DEEPSEEK_API_KEY"
DEFAULT_UPSTREAM_AUTH_HEADER_NAME = "Authorization"
DEFAULT_UPSTREAM_AUTH_SCHEME = "Bearer"
DEFAULT_TIMEOUT_SECONDS = 120.0
DEFAULT_READ_CHUNK_BYTES = 1024
DEFAULT_RESPONSE_CHUNK_DELAY_SECONDS = 0.02
DEFAULT_MODE = "forward"
MODE_FORWARD = "forward"
MODE_LOCAL_STREAM = "local-stream"
DEFAULT_LOCAL_STREAM_RESPONSE_TEXT = "ACTRAIL_XIAOO_HTTP_PROXY_OK"


@dataclass(frozen=True)
class ProxyConfig:
    mode: str
    bind_host: str
    bind_port: int
    upstream_base_url: str
    upstream_api_key_env: str
    upstream_auth_header_name: str
    upstream_auth_scheme: str
    timeout_seconds: float
    read_chunk_bytes: int
    response_chunk_delay_seconds: float
    local_stream_response_text: str


def main() -> int:
    config = parse_args()
    upstream = urlsplit(config.upstream_base_url)
    if config.mode == MODE_FORWARD:
        if upstream.scheme != "https":
            raise RuntimeError("upstream_base_url must use https")
        if not upstream.netloc:
            raise RuntimeError("upstream_base_url must include a host")
        if upstream.query or upstream.fragment:
            raise RuntimeError("upstream_base_url must not include query or fragment")
        if not os.environ.get(config.upstream_api_key_env):
            raise RuntimeError(f"missing upstream API key env {config.upstream_api_key_env}")

    server = ThreadingHTTPServer(
        (config.bind_host, config.bind_port),
        make_handler(config),
    )
    host, port = server.server_address
    print(f"proxy_base_url=http://{host}:{port}", flush=True)
    try:
        server.serve_forever()
    finally:
        server.server_close()
    return 0


def parse_args() -> ProxyConfig:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    parser.add_argument(
        "--mode",
        choices=(MODE_FORWARD, MODE_LOCAL_STREAM),
        default=DEFAULT_MODE,
        help="proxy behavior",
    )
    parser.add_argument("--bind-host", default=DEFAULT_BIND_HOST, help="local bind host")
    parser.add_argument("--bind-port", type=int, default=DEFAULT_BIND_PORT, help="local bind port")
    parser.add_argument(
        "--upstream-base-url",
        default=DEFAULT_UPSTREAM_BASE_URL,
        help="HTTPS upstream provider base URL",
    )
    parser.add_argument(
        "--upstream-api-key-env",
        default=DEFAULT_UPSTREAM_API_KEY_ENV,
        help="environment variable containing the upstream provider API key",
    )
    parser.add_argument(
        "--upstream-auth-header-name",
        default=DEFAULT_UPSTREAM_AUTH_HEADER_NAME,
        help="upstream auth header name",
    )
    parser.add_argument(
        "--upstream-auth-scheme",
        default=DEFAULT_UPSTREAM_AUTH_SCHEME,
        help="upstream auth scheme, or none to send the raw key",
    )
    parser.add_argument(
        "--timeout-seconds",
        type=float,
        default=DEFAULT_TIMEOUT_SECONDS,
        help="upstream request timeout",
    )
    parser.add_argument(
        "--read-chunk-bytes",
        type=int,
        default=DEFAULT_READ_CHUNK_BYTES,
        help="upstream response read chunk size",
    )
    parser.add_argument(
        "--response-chunk-delay-seconds",
        type=float,
        default=DEFAULT_RESPONSE_CHUNK_DELAY_SECONDS,
        help="delay after each response chunk written to the local client",
    )
    parser.add_argument(
        "--local-stream-response-text",
        default=DEFAULT_LOCAL_STREAM_RESPONSE_TEXT,
        help="assistant content emitted by local-stream mode",
    )
    args = parser.parse_args()
    if args.bind_port < 0:
        raise RuntimeError("--bind-port must be non-negative")
    if args.timeout_seconds <= 0:
        raise RuntimeError("--timeout-seconds must be positive")
    if args.read_chunk_bytes <= 0:
        raise RuntimeError("--read-chunk-bytes must be positive")
    if args.response_chunk_delay_seconds < 0:
        raise RuntimeError("--response-chunk-delay-seconds must be non-negative")
    if args.mode == MODE_LOCAL_STREAM and not args.local_stream_response_text:
        raise RuntimeError("--local-stream-response-text must be non-empty")
    return ProxyConfig(
        mode=args.mode,
        bind_host=args.bind_host,
        bind_port=args.bind_port,
        upstream_base_url=args.upstream_base_url,
        upstream_api_key_env=args.upstream_api_key_env,
        upstream_auth_header_name=args.upstream_auth_header_name,
        upstream_auth_scheme=args.upstream_auth_scheme,
        timeout_seconds=args.timeout_seconds,
        read_chunk_bytes=args.read_chunk_bytes,
        response_chunk_delay_seconds=args.response_chunk_delay_seconds,
        local_stream_response_text=args.local_stream_response_text,
    )


def make_handler(config: ProxyConfig):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def do_POST(self) -> None:
            try:
                forward_post(self, config)
            except Exception as error:
                print(f"proxy_error={error}", file=sys.stderr, flush=True)
                if not self.wfile.closed:
                    self.send_error(502, "upstream proxy failure")

        def log_message(self, *_args) -> None:
            return

    return Handler


def forward_post(handler: BaseHTTPRequestHandler, config: ProxyConfig) -> None:
    body = read_request_body(handler)
    if config.mode == MODE_LOCAL_STREAM:
        respond_local_stream(handler, config, body)
        return

    upstream = urlsplit(config.upstream_base_url)
    target = upstream_path(upstream.path, handler.path)
    connection = http.client.HTTPSConnection(
        upstream.hostname,
        upstream.port,
        timeout=config.timeout_seconds,
    )
    try:
        connection.request(
            "POST",
            target,
            body=body,
            headers=upstream_headers(handler, config, upstream.netloc, len(body)),
        )
        response = connection.getresponse()
        handler.send_response(response.status, response.reason)
        for name, value in response.getheaders():
            if response_header_allowed(name):
                handler.send_header(name, value)
        handler.send_header("Transfer-Encoding", "chunked")
        handler.send_header("Connection", "close")
        handler.end_headers()
        response_bytes = stream_response(
            response,
            handler,
            config.read_chunk_bytes,
            config.response_chunk_delay_seconds,
        )
        print(
            "proxy_forward "
            f"method=POST path={handler.path} status={response.status} "
            f"request_bytes={len(body)} response_bytes={response_bytes}",
            flush=True,
        )
        handler.close_connection = True
    finally:
        connection.close()


def respond_local_stream(
    handler: BaseHTTPRequestHandler,
    config: ProxyConfig,
    body: bytes,
) -> None:
    request = parse_json_body(body)
    model = str(request.get("model") or "actrail-local-stream")
    chunks = local_stream_chunks(model, config.local_stream_response_text)
    handler.send_response(200, "OK")
    handler.send_header("Content-Type", "text/event-stream")
    handler.send_header("Cache-Control", "no-cache")
    handler.send_header("Transfer-Encoding", "chunked")
    handler.send_header("Connection", "close")
    handler.end_headers()
    response_bytes = 0
    for chunk in chunks:
        payload = (
            "data: "
            + json.dumps(chunk, ensure_ascii=False, separators=(",", ":"))
            + "\n\n"
        ).encode("utf-8")
        write_chunked_payload(handler, payload)
        handler.wfile.flush()
        response_bytes += len(payload)
        if config.response_chunk_delay_seconds > 0:
            time.sleep(config.response_chunk_delay_seconds)
    write_chunked_terminator(handler)
    handler.wfile.flush()
    handler.close_connection = True
    print(
        "proxy_local_stream "
        f"method=POST path={handler.path} status=200 "
        f"request_bytes={len(body)} response_bytes={response_bytes}",
        flush=True,
    )


def parse_json_body(body: bytes) -> dict:
    try:
        value = json.loads(body.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise RuntimeError(f"invalid JSON request body: {error}") from error
    if not isinstance(value, dict):
        raise RuntimeError("JSON request body must be an object")
    return value


def local_stream_chunks(model: str, response_text: str) -> list[dict]:
    chunks = [
        {
            "id": "chatcmpl-actrail-local",
            "object": "chat.completion.chunk",
            "created": 0,
            "model": model,
            "choices": [
                {
                    "index": 0,
                    "delta": {"content": part},
                    "finish_reason": None,
                }
            ],
        }
        for part in split_response_text(response_text)
    ]
    chunks.append(
        {
            "id": "chatcmpl-actrail-local",
            "object": "chat.completion.chunk",
            "created": 0,
            "model": model,
            "choices": [
                {
                    "index": 0,
                    "delta": {},
                    "finish_reason": "stop",
                }
            ],
            "usage": {
                "prompt_tokens": 1,
                "completion_tokens": len(response_text),
                "total_tokens": len(response_text) + 1,
            },
        }
    )
    return chunks


def split_response_text(response_text: str) -> list[str]:
    if len(response_text) <= 8:
        return [response_text]
    step = max(1, len(response_text) // 3)
    return [
        response_text[index : index + step]
        for index in range(0, len(response_text), step)
    ]


def read_request_body(handler: BaseHTTPRequestHandler) -> bytes:
    raw_length = handler.headers.get("Content-Length")
    if raw_length is None:
        raise RuntimeError("request is missing Content-Length")
    try:
        length = int(raw_length)
    except ValueError as error:
        raise RuntimeError(f"invalid Content-Length: {raw_length}") from error
    if length < 0:
        raise RuntimeError("Content-Length must be non-negative")
    return handler.rfile.read(length)


def upstream_headers(
    handler: BaseHTTPRequestHandler,
    config: ProxyConfig,
    upstream_host: str,
    content_length: int,
) -> dict[str, str]:
    headers: dict[str, str] = {
        "Host": upstream_host,
        "Content-Length": str(content_length),
    }
    for name, value in handler.headers.items():
        lower = name.lower()
        if lower in HOP_BY_HOP_HEADERS or lower in {"host", "authorization", "content-length"}:
            continue
        headers[name] = value
    api_key = os.environ[config.upstream_api_key_env]
    headers[config.upstream_auth_header_name] = auth_header_value(
        config.upstream_auth_scheme,
        api_key,
    )
    return headers


def auth_header_value(scheme: str, api_key: str) -> str:
    return api_key if scheme == "none" else f"{scheme} {api_key}"


def upstream_path(base_path: str, incoming_path: str) -> str:
    incoming = incoming_path if incoming_path.startswith("/") else f"/{incoming_path}"
    prefix = base_path.rstrip("/")
    if not prefix:
        return incoming
    return f"{prefix}{incoming}"


def response_header_allowed(name: str) -> bool:
    lower = name.lower()
    return lower not in HOP_BY_HOP_HEADERS and lower != "content-length"


def stream_response(
    response: http.client.HTTPResponse,
    handler: BaseHTTPRequestHandler,
    chunk_bytes: int,
    chunk_delay_seconds: float,
) -> int:
    total = 0
    while True:
        chunk = response.read(chunk_bytes)
        if not chunk:
            write_chunked_terminator(handler)
            handler.wfile.flush()
            return total
        write_chunked_payload(handler, chunk)
        handler.wfile.flush()
        total += len(chunk)
        if chunk_delay_seconds > 0:
            time.sleep(chunk_delay_seconds)


def write_chunked_payload(handler: BaseHTTPRequestHandler, payload: bytes) -> None:
    handler.wfile.write(f"{len(payload):x}\r\n".encode("ascii"))
    handler.wfile.write(payload)
    handler.wfile.write(b"\r\n")


def write_chunked_terminator(handler: BaseHTTPRequestHandler) -> None:
    handler.wfile.write(b"0\r\n\r\n")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"LLM HTTP proxy failed: {error}", file=sys.stderr)
        raise SystemExit(1)
