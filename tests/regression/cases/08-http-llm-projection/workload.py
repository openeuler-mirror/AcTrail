#!/usr/bin/env python3
"""Local HTTP workload that emits an OpenAI-style LLM request body."""

from __future__ import annotations

import argparse
import http.client
import json
import socket
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def main() -> int:
    args = parse_args()
    if args.response_read_chunk_bytes <= 0:
        raise RuntimeError("--response-read-chunk-bytes must be positive")
    server = ThreadingHTTPServer(
        (args.bind_host, args.bind_port), response_handler(args.response_text)
    )
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        send_llm_request(args, server.server_port)
    finally:
        server.shutdown()
        thread.join(timeout=args.timeout_seconds)
    print("llm projection workload complete", flush=True)
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", required=True)
    parser.add_argument("--marker", required=True)
    parser.add_argument("--path", required=True)
    parser.add_argument("--bind-host", required=True)
    parser.add_argument("--bind-port", type=int, required=True)
    parser.add_argument("--response-text", required=True)
    parser.add_argument("--timeout-seconds", type=float, required=True)
    parser.add_argument(
        "--request-write-mode",
        choices=["http-client", "single-syscall"],
        required=True,
    )
    parser.add_argument("--host-header", required=True)
    parser.add_argument("--content-type", required=True)
    parser.add_argument("--response-read-chunk-bytes", type=int, required=True)
    parser.add_argument("--request-padding-bytes", type=int, required=True)
    return parser.parse_args()


def response_handler(response_text: str):
    response_bytes = response_text.encode("utf-8")

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:
            length = int(self.headers.get("content-length", "0"))
            self.rfile.read(length)
            self.send_response(200)
            self.send_header("Content-Length", str(len(response_bytes)))
            self.end_headers()
            self.wfile.write(response_bytes)

        def log_message(self, *_args) -> None:
            return

    return Handler


def send_llm_request(args: argparse.Namespace, port: int) -> None:
    body = json.dumps(
        {
            "model": args.model,
            "messages": [{"role": "user", "content": args.marker}],
            "padding": "x" * args.request_padding_bytes,
        }
    ).encode("utf-8")
    if args.request_write_mode == "single-syscall":
        send_single_syscall_request(args, port, body)
        return
    conn = http.client.HTTPConnection(args.bind_host, port, timeout=args.timeout_seconds)
    conn.request(
        "POST",
        args.path,
        body,
        {
            "Content-Type": args.content_type,
            "Host": args.host_header,
        },
    )
    response = conn.getresponse()
    response_body = response.read().decode("utf-8")
    print(f"http_status={response.status}", flush=True)
    print(f"http_response={response_body}", flush=True)
    if response.status != 200:
        raise RuntimeError(f"unexpected HTTP status {response.status}")
    if response_body != args.response_text:
        raise RuntimeError("unexpected HTTP response body")


def send_single_syscall_request(args: argparse.Namespace, port: int, body: bytes) -> None:
    request = (
        f"POST {args.path} HTTP/1.1\r\n"
        f"Host: {args.host_header}\r\n"
        f"Content-Type: {args.content_type}\r\n"
        f"Content-Length: {len(body)}\r\n"
        "Connection: close\r\n"
        "\r\n"
    ).encode("ascii") + body
    with socket.create_connection((args.bind_host, port), timeout=args.timeout_seconds) as client:
        client.settimeout(args.timeout_seconds)
        written = client.send(request)
        if written != len(request):
            raise RuntimeError(f"single syscall request write was partial: {written}/{len(request)}")
        response = read_http_response(client, args.response_read_chunk_bytes)
    print(f"http_status={response.status}", flush=True)
    print(f"http_response={response.body}", flush=True)
    if response.status != 200:
        raise RuntimeError(f"unexpected HTTP status {response.status}")
    if response.body != args.response_text:
        raise RuntimeError("unexpected HTTP response body")


class HttpResponse:
    def __init__(self, status: int, body: str) -> None:
        self.status = status
        self.body = body


def read_http_response(client: socket.socket, chunk_bytes: int) -> HttpResponse:
    chunks = []
    while True:
        chunk = client.recv(chunk_bytes)
        if not chunk:
            break
        chunks.append(chunk)
    raw = b"".join(chunks)
    header, separator, body = raw.partition(b"\r\n\r\n")
    if not separator:
        raise RuntimeError("HTTP response did not contain a header/body separator")
    status_line = header.decode("iso-8859-1").splitlines()[0]
    parts = status_line.split()
    if len(parts) < 2 or not parts[1].isdigit():
        raise RuntimeError(f"invalid HTTP response status line: {status_line}")
    return HttpResponse(int(parts[1]), body.decode("utf-8"))


if __name__ == "__main__":
    raise SystemExit(main())
