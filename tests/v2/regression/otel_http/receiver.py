from __future__ import annotations

from http import HTTPStatus

import copy
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

from .constants import CASE_CONSTANTS


class OtlpHttpReceiver:
    """Small local OTLP/HTTP JSON receiver used by this black-box case."""

    _MAX_REQUEST_BYTES = CASE_CONSTANTS.max_request_bytes

    #: Credential header a real ingest endpoint uses to attribute the trace.
    CREDENTIAL_HEADER = "x-witty-api-key"

    def __init__(self, host: str = CASE_CONSTANTS.init_host, port: int = CASE_CONSTANTS.init_port):
        self._documents: list[dict[str, Any]] = []
        self._credentials: list[str | None] = []
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
        thread.join(timeout=CASE_CONSTANTS.stop_timeout)
        self._thread = None

    def documents(self) -> list[dict[str, Any]]:
        with self._lock:
            return copy.deepcopy(self._documents)

    def credentials(self) -> list[str | None]:
        """The credential header seen on each accepted request, in order."""
        with self._lock:
            return list(self._credentials)

    def handle(self, handler: BaseHTTPRequestHandler) -> None:
        if handler.path != "/v1/traces":
            self._send_json(handler, HTTPStatus.NOT_FOUND, {"error": "not found"})
            return
        if handler.headers.get_content_type() != "application/json":
            self._send_json(handler, HTTPStatus.UNSUPPORTED_MEDIA_TYPE, {"error": "invalid content type"})
            return
        try:
            length = int(handler.headers.get("Content-Length", ""))
        except ValueError:
            self._send_json(handler, HTTPStatus.LENGTH_REQUIRED, {"error": "invalid content length"})
            return
        if length <= 0 or length > self._MAX_REQUEST_BYTES:
            self._send_json(handler, HTTPStatus.REQUEST_ENTITY_TOO_LARGE, {"error": "invalid request size"})
            return
        raw = handler.rfile.read(length)
        try:
            document = json.loads(raw)
        except (UnicodeDecodeError, json.JSONDecodeError):
            self._send_json(handler, HTTPStatus.BAD_REQUEST, {"error": "invalid JSON"})
            return
        if not isinstance(document, dict) or not isinstance(
            document.get("resourceSpans"), list
        ):
            self._send_json(handler, HTTPStatus.BAD_REQUEST, {"error": "invalid OTLP JSON"})
            return
        with self._lock:
            self._documents.append(document)
            self._credentials.append(handler.headers.get(self.CREDENTIAL_HEADER))
        # An empty ExportTraceServiceResponse is a valid OTLP success body.
        self._send_json(handler, HTTPStatus.OK, {})

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
