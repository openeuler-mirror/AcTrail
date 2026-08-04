from __future__ import annotations

import argparse
import copy
import json
import threading
import time
from collections import deque
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any


class JsonRpcOtelReceiver:
    _MAX_REQUEST_BYTES = 16 * 1024 * 1024

    def __init__(
        self,
        method: str,
        host: str = "127.0.0.1",
        port: int = 0,
    ):
        self._method = method
        self._documents: list[dict[str, Any]] = []
        self._request_ids: list[int] = []
        self._lock = threading.Lock()
        self._failures_remaining = 0
        self._response_delays: deque[float] = deque()
        self._injected_failures = 0
        self._injected_response_delays = 0
        self._server = _JsonRpcServer((host, port), self)
        self._thread: threading.Thread | None = None

    @property
    def endpoint(self) -> str:
        host, port = self._server.server_address[:2]
        return f"http://{host}:{port}/rpc"

    @property
    def injected_failures(self) -> int:
        with self._lock:
            return self._injected_failures

    @property
    def injected_response_delays(self) -> int:
        with self._lock:
            return self._injected_response_delays

    def start(self) -> None:
        if self._thread is not None:
            return
        self._thread = threading.Thread(
            target=self._server.serve_forever,
            name="otel-json-rpc-receiver",
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

    def fail_next_requests(self, count: int) -> None:
        if count <= 0:
            raise ValueError("failure count must be positive")
        with self._lock:
            self._failures_remaining += count

    def delay_next_responses(
        self,
        delay_seconds: float,
        count: int = 1,
    ) -> None:
        if delay_seconds <= 0:
            raise ValueError("response delay must be positive")
        if count <= 0:
            raise ValueError("response delay count must be positive")
        with self._lock:
            self._response_delays.extend(delay_seconds for _ in range(count))

    def documents(self) -> list[dict[str, Any]]:
        with self._lock:
            return copy.deepcopy(self._documents)

    def request_ids(self) -> list[int]:
        with self._lock:
            return list(self._request_ids)

    def handle(self, handler: BaseHTTPRequestHandler) -> None:
        if handler.path != "/rpc":
            self._send_json(handler, 404, {"error": "not found"})
            return
        try:
            length = int(handler.headers.get("Content-Length", ""))
        except ValueError:
            self._send_json(handler, 411, {"error": "invalid content length"})
            return
        if length <= 0 or length > self._MAX_REQUEST_BYTES:
            self._send_json(handler, 413, {"error": "invalid request size"})
            return
        if handler.headers.get_content_type() != "application/json":
            self._send_json(handler, 415, {"error": "invalid content type"})
            return
        raw = handler.rfile.read(length)
        try:
            request = json.loads(raw)
        except (UnicodeDecodeError, json.JSONDecodeError):
            self._send_json(handler, 400, {"error": "invalid JSON"})
            return
        validation_error = self._validate_request(request)
        if validation_error is not None:
            self._send_json(
                handler,
                400,
                {
                    "jsonrpc": "2.0",
                    "id": request.get("id") if isinstance(request, dict) else None,
                    "error": {"code": -32600, "message": validation_error},
                },
            )
            return
        with self._lock:
            self._request_ids.append(request["id"])
            if self._failures_remaining > 0:
                self._failures_remaining -= 1
                self._injected_failures += 1
                should_fail = True
                response_delay = None
            else:
                self._documents.append(request["params"])
                should_fail = False
                if self._response_delays:
                    response_delay = self._response_delays.popleft()
                    self._injected_response_delays += 1
                else:
                    response_delay = None
        if should_fail:
            self._send_json(
                handler,
                503,
                {"error": "injected retryable failure"},
                close_connection=True,
            )
            return
        if response_delay is not None:
            time.sleep(response_delay)
        self._send_json(
            handler,
            200,
            {
                "jsonrpc": "2.0",
                "id": request["id"],
                "result": {"accepted": 1},
            },
            close_connection=response_delay is not None,
            ignore_disconnect=response_delay is not None,
        )

    def _validate_request(self, request: Any) -> str | None:
        if not isinstance(request, dict):
            return "request must be an object"
        if request.get("jsonrpc") != "2.0":
            return "jsonrpc must be 2.0"
        if request.get("method") != self._method:
            return f"method must be {self._method}"
        request_id = request.get("id")
        if (
            not isinstance(request_id, int)
            or isinstance(request_id, bool)
            or request_id <= 0
        ):
            return "id must be a positive integer"
        params = request.get("params")
        if not isinstance(params, dict) or not isinstance(
            params.get("resourceSpans"),
            list,
        ):
            return "params must be an OTLP JSON object"
        return None

    @staticmethod
    def _send_json(
        handler: BaseHTTPRequestHandler,
        status: int,
        document: dict[str, Any],
        *,
        close_connection: bool = False,
        ignore_disconnect: bool = False,
    ) -> None:
        body = json.dumps(document, separators=(",", ":")).encode("utf-8")
        try:
            handler.send_response(status)
            handler.send_header("Content-Type", "application/json")
            handler.send_header("Content-Length", str(len(body)))
            if close_connection:
                handler.send_header("Connection", "close")
                handler.close_connection = True
            handler.end_headers()
            handler.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            if not ignore_disconnect:
                raise


class _JsonRpcServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(
        self,
        address: tuple[str, int],
        receiver: JsonRpcOtelReceiver,
    ):
        self.receiver = receiver
        super().__init__(address, _JsonRpcRequestHandler)


class _JsonRpcRequestHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_POST(self) -> None:
        server = self.server
        if not isinstance(server, _JsonRpcServer):
            self.send_error(500)
            return
        server.receiver.handle(self)

    def log_message(self, format: str, *args: Any) -> None:
        del format, args


class JsonRpcOtelReceiverConsole:
    _POLL_INTERVAL_SECONDS = 0.1

    def __init__(
        self,
        receiver: JsonRpcOtelReceiver,
        output_path: Path,
    ):
        self._receiver = receiver
        self._output_path = output_path
        self._observed_request_count = 0
        self._written_document_count = 0

    def run(self) -> int:
        self._output_path.parent.mkdir(parents=True, exist_ok=True)
        self._output_path.write_text("", encoding="utf-8")
        self._receiver.start()
        print(f"endpoint={self._receiver.endpoint}", flush=True)
        print(f"output={self._output_path}", flush=True)
        try:
            while True:
                self._report_requests()
                self._write_documents()
                time.sleep(self._POLL_INTERVAL_SECONDS)
        except KeyboardInterrupt:
            return 0
        finally:
            self._receiver.stop()
            self._report_requests()
            self._write_documents()

    def _report_requests(self) -> None:
        request_ids = self._receiver.request_ids()
        for request_id in request_ids[self._observed_request_count :]:
            print(f"request_id={request_id}", flush=True)
        self._observed_request_count = len(request_ids)

    def _write_documents(self) -> None:
        documents = self._receiver.documents()
        pending = documents[self._written_document_count :]
        if not pending:
            return
        with self._output_path.open("a", encoding="utf-8") as output:
            for document in pending:
                output.write(json.dumps(document, separators=(",", ":")))
                output.write("\n")
        self._written_document_count = len(documents)
        print(
            f"received_documents={self._written_document_count}",
            flush=True,
        )


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Receive OTEL JSON through JSON-RPC 2.0 over HTTP.",
    )
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--method", default="otel.export")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--fail-next", type=int, default=0)
    parser.add_argument("--delay-next-ms", type=int, default=0)
    arguments = parser.parse_args()
    if arguments.port < 0 or arguments.port > 65535:
        parser.error("--port must be between 0 and 65535")
    if arguments.fail_next < 0:
        parser.error("--fail-next must not be negative")
    if arguments.delay_next_ms < 0:
        parser.error("--delay-next-ms must not be negative")

    receiver = JsonRpcOtelReceiver(
        arguments.method,
        arguments.host,
        arguments.port,
    )
    if arguments.fail_next:
        receiver.fail_next_requests(arguments.fail_next)
    if arguments.delay_next_ms:
        receiver.delay_next_responses(arguments.delay_next_ms / 1000)
    return JsonRpcOtelReceiverConsole(receiver, arguments.output).run()


if __name__ == "__main__":
    raise SystemExit(main())
