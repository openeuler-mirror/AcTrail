from __future__ import annotations

import copy
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any


class OtlpHttpReceiver:
    """Small local OTLP/HTTP JSON receiver used by this black-box case."""

    _MAX_REQUEST_BYTES = 16 * 1024 * 1024

    def __init__(self, host: str = "127.0.0.1", port: int = 0):
        self._documents: list[dict[str, Any]] = []
        self._lock = threading.Lock()
        self._server = _OtlpHttpServer((host, port), self)
        self._thread: threading.Thread | None = None

    @property
    def endpoint(self) -> str:
        host, port = self._server.server_address[:2]
        return f"http://{host}:{port}/v1/traces"

    def start(self) -> None:
        if self._thread is not None:
            return
        self._thread = threading.Thread(
            target=self._server.serve_forever,
            name="otel-http-v2-receiver",
            daemon=True,
        )
        self._thread.start()

    def stop(self) -> None:
        thread = self._thread
        if thread is None:
            self._server.server_close()
            return
        self._server.shutdown()
        self._server.server_close()
        thread.join(timeout=10)
        self._thread = None

    def documents(self) -> list[dict[str, Any]]:
        with self._lock:
            return copy.deepcopy(self._documents)

    def handle(self, handler: BaseHTTPRequestHandler) -> None:
        if handler.path != "/v1/traces":
            self._send_json(handler, 404, {"error": "not found"})
            return
        if handler.headers.get_content_type() != "application/json":
            self._send_json(handler, 415, {"error": "invalid content type"})
            return
        try:
            length = int(handler.headers.get("Content-Length", ""))
        except ValueError:
            self._send_json(handler, 411, {"error": "invalid content length"})
            return
        if length <= 0 or length > self._MAX_REQUEST_BYTES:
            self._send_json(handler, 413, {"error": "invalid request size"})
            return
        raw = handler.rfile.read(length)
        try:
            document = json.loads(raw)
        except (UnicodeDecodeError, json.JSONDecodeError):
            self._send_json(handler, 400, {"error": "invalid JSON"})
            return
        if not isinstance(document, dict) or not isinstance(
            document.get("resourceSpans"), list
        ):
            self._send_json(handler, 400, {"error": "invalid OTLP JSON"})
            return
        with self._lock:
            self._documents.append(document)
        # An empty ExportTraceServiceResponse is a valid OTLP success body.
        self._send_json(handler, 200, {})

    @staticmethod
    def _send_json(
        handler: BaseHTTPRequestHandler,
        status: int,
        document: dict[str, Any],
    ) -> None:
        body = json.dumps(document, separators=(",", ":")).encode("utf-8")
        handler.send_response(status)
        handler.send_header("Content-Type", "application/json")
        handler.send_header("Content-Length", str(len(body)))
        handler.end_headers()
        handler.wfile.write(body)


class _OtlpHttpServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(
        self,
        address: tuple[str, int],
        receiver: OtlpHttpReceiver,
    ):
        self.receiver = receiver
        super().__init__(address, _OtlpHttpRequestHandler)


class _OtlpHttpRequestHandler(BaseHTTPRequestHandler):
    server: _OtlpHttpServer

    def do_POST(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
        self.server.receiver.handle(self)

    def log_message(self, format: str, *args: object) -> None:
        del format, args
