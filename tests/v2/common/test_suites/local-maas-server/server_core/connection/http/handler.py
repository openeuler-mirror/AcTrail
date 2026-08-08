from __future__ import annotations

import hmac
import json
import socket
import ssl
import sys
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler
from typing import Any
from urllib.parse import urlsplit

from protocol import ProtocolResponse
from server_core.application import LocalMaaSApplication
from server_core.connection.interface import ConnectionServer
from utils.json import StrictJsonDecoder, StrictJsonError


class ConnectionRequestError(RuntimeError):
    def __init__(self, status: int, code: str, message: str):
        super().__init__(message)
        self.status = status
        self.code = code


class HTTPRequestHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "LocalMaaS/1"

    @property
    def connection_server(self) -> ConnectionServer:
        server = self.server
        if not isinstance(server, ConnectionServer):
            raise RuntimeError("handler is attached to the wrong server type")
        return server

    @property
    def application(self) -> LocalMaaSApplication:
        return self.connection_server.application

    def setup(self) -> None:
        super().setup()
        self.connection.settimeout(
            self.connection_server.config.request_timeout_seconds
        )

    def handle(self) -> None:
        try:
            super().handle()
        except (
            BrokenPipeError,
            ConnectionResetError,
            socket.timeout,
            ssl.SSLError,
        ):
            self.close_connection = True

    def handle_one_request(self) -> None:
        self._response_started = False
        super().handle_one_request()

    def do_GET(self) -> None:
        path = self._normalized_path()
        if self.application.supports_health_path(path):
            self._send_response(
                self._json_response(
                    HTTPStatus.OK,
                    self.application.health(self.connection_server.origin),
                )
            )
            return
        if self.application.supports_models_path(path):
            self._send_response(
                self._json_response(
                    HTTPStatus.OK,
                    self.application.models(),
                )
            )
            return
        self._send_response(
            self.application.generic_error(
                HTTPStatus.NOT_FOUND,
                "route_not_found",
                f"no GET endpoint at {path}",
            )
        )

    def do_POST(self) -> None:
        path = self._normalized_path()
        if not self.application.supports_path(path):
            self._send_response(
                self.application.generic_error(
                    HTTPStatus.NOT_FOUND,
                    "route_not_found",
                    f"no MaaS endpoint at {path}",
                ),
                close_connection=True,
            )
            return
        try:
            self._verify_api_key()
            document = self._read_json_request()
            result = self.application.handle_post(path, document)
            completed = self._send_response(
                result.response,
                close_connection=result.response.status >= 400,
            )
            if completed:
                self.application.record_completed(result)
        except ConnectionRequestError as error:
            self._send_response(
                self.application.error_for_path(
                    path, error.status, error.code, str(error)
                ),
                close_connection=True,
            )
        except (BrokenPipeError, ConnectionResetError):
            self.close_connection = True
        except socket.timeout:
            self._send_response(
                self.application.error_for_path(
                    path,
                    HTTPStatus.REQUEST_TIMEOUT,
                    "request_timeout",
                    "request exceeded the configured timeout",
                ),
                close_connection=True,
            )
        except Exception as error:
            print(
                f"local_maas_request_error={type(error).__name__}: {error}",
                file=sys.stderr,
                flush=True,
            )
            if not self._response_started:
                self._send_response(
                    self.application.error_for_path(
                        path,
                        HTTPStatus.INTERNAL_SERVER_ERROR,
                        "internal_error",
                        "request failed inside the local MaaS server",
                    ),
                    close_connection=True,
                )
            else:
                self.close_connection = True

    def log_message(self, *_args: Any) -> None:
        return

    def _read_json_request(self) -> dict[str, Any]:
        body = self._read_request_body()
        try:
            document = StrictJsonDecoder().decode_utf8(body)
        except StrictJsonError as error:
            raise ConnectionRequestError(
                HTTPStatus.BAD_REQUEST,
                "invalid_json",
                str(error),
            ) from error
        if not isinstance(document, dict):
            raise ConnectionRequestError(
                HTTPStatus.BAD_REQUEST,
                "invalid_request",
                "request body must be a JSON object",
            )
        return document

    def _read_request_body(self) -> bytes:
        transfer_encoding = self.headers.get("Transfer-Encoding", "")
        encodings = {
            value.strip().lower()
            for value in transfer_encoding.split(",")
            if value.strip()
        }
        raw_length = self.headers.get("Content-Length")
        if encodings:
            if encodings != {"chunked"}:
                raise ConnectionRequestError(
                    HTTPStatus.BAD_REQUEST,
                    "invalid_transfer_encoding",
                    "only chunked Transfer-Encoding is supported",
                )
            if raw_length is not None:
                raise ConnectionRequestError(
                    HTTPStatus.BAD_REQUEST,
                    "ambiguous_body_length",
                    "request cannot contain both Content-Length and chunked "
                    "encoding",
                )
            return self._read_chunked_body()
        if raw_length is None:
            raise ConnectionRequestError(
                HTTPStatus.LENGTH_REQUIRED,
                "content_length_required",
                "request must contain Content-Length or use chunked encoding",
            )
        try:
            length = int(raw_length)
        except ValueError as error:
            raise ConnectionRequestError(
                HTTPStatus.BAD_REQUEST,
                "invalid_content_length",
                "Content-Length must be an integer",
            ) from error
        if length < 0:
            raise ConnectionRequestError(
                HTTPStatus.BAD_REQUEST,
                "invalid_content_length",
                "Content-Length must be non-negative",
            )
        limit = self.connection_server.config.max_request_bytes
        if length > limit:
            raise ConnectionRequestError(
                HTTPStatus.REQUEST_ENTITY_TOO_LARGE,
                "request_too_large",
                f"request exceeds the configured {limit}-byte limit",
            )
        body = self.rfile.read(length)
        if len(body) != length:
            raise ConnectionRequestError(
                HTTPStatus.BAD_REQUEST,
                "incomplete_request_body",
                f"request ended after {len(body)} of {length} bytes",
            )
        return body

    def _read_chunked_body(self) -> bytes:
        body = bytearray()
        limit = self.connection_server.config.max_request_bytes
        while True:
            size_line = self.rfile.readline(128)
            if not size_line.endswith(b"\n"):
                raise ConnectionRequestError(
                    HTTPStatus.BAD_REQUEST,
                    "invalid_chunk",
                    "chunk size line is missing its terminator or is too long",
                )
            try:
                raw_size = size_line.split(b";", 1)[0].strip()
                size = int(raw_size, 16)
            except ValueError as error:
                raise ConnectionRequestError(
                    HTTPStatus.BAD_REQUEST,
                    "invalid_chunk",
                    "chunk size must be hexadecimal",
                ) from error
            if size < 0:
                raise ConnectionRequestError(
                    HTTPStatus.BAD_REQUEST,
                    "invalid_chunk",
                    "chunk size must be non-negative",
                )
            if size == 0:
                trailer = self.rfile.readline(2)
                if trailer not in {b"\r\n", b"\n"}:
                    raise ConnectionRequestError(
                        HTTPStatus.BAD_REQUEST,
                        "unsupported_chunk_trailer",
                        "chunked request trailers are not supported",
                    )
                return bytes(body)
            if len(body) + size > limit:
                raise ConnectionRequestError(
                    HTTPStatus.REQUEST_ENTITY_TOO_LARGE,
                    "request_too_large",
                    f"request exceeds the configured {limit}-byte limit",
                )
            chunk = self.rfile.read(size)
            terminator = self.rfile.read(2)
            if len(chunk) != size or terminator != b"\r\n":
                raise ConnectionRequestError(
                    HTTPStatus.BAD_REQUEST,
                    "invalid_chunk",
                    "chunk body is incomplete or missing CRLF",
                )
            body.extend(chunk)

    def _verify_api_key(self) -> None:
        expected = self.connection_server.config.api_key
        if expected is None:
            return
        supplied: list[str] = []
        authorization = self.headers.get("Authorization")
        if authorization is not None:
            scheme, separator, value = authorization.partition(" ")
            if separator and scheme.lower() == "bearer":
                supplied.append(value)
        anthropic_key = self.headers.get("X-Api-Key")
        if anthropic_key is not None:
            supplied.append(anthropic_key)
        expected_bytes = expected.encode("utf-8")
        if not any(
            hmac.compare_digest(value.encode("utf-8"), expected_bytes)
            for value in supplied
        ):
            raise ConnectionRequestError(
                HTTPStatus.UNAUTHORIZED,
                "invalid_api_key",
                "request did not provide the configured local API key",
            )

    def _send_response(
        self,
        response: ProtocolResponse,
        *,
        close_connection: bool = False,
    ) -> bool:
        if self._response_started:
            self.close_connection = True
            return False
        if response.body is not None:
            return self._send_body(response, close_connection)
        return self._send_stream(response)

    def _send_body(
        self, response: ProtocolResponse, close_connection: bool
    ) -> bool:
        body = response.body
        if body is None:
            raise RuntimeError("direct response has no body")
        self.send_response(response.status)
        self.send_header("Content-Type", response.media_type)
        self.send_header("Content-Length", str(len(body)))
        if close_connection:
            self.send_header("Connection", "close")
        self.end_headers()
        self._response_started = True
        self.wfile.write(body)
        self.wfile.flush()
        if close_connection:
            self.close_connection = True
        return True

    def _send_stream(self, response: ProtocolResponse) -> bool:
        frames = response.frames
        if frames is None:
            raise RuntimeError("stream response has no frames")
        self.send_response(response.status)
        self.send_header("Content-Type", response.media_type)
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Transfer-Encoding", "chunked")
        self.end_headers()
        self._response_started = True
        try:
            for frame in frames:
                self._write_chunk(frame.payload)
                self.wfile.flush()
            self.wfile.write(b"0\r\n\r\n")
            self.wfile.flush()
            return True
        except (BrokenPipeError, ConnectionResetError, socket.timeout):
            self.close_connection = True
            return False

    def _write_chunk(self, payload: bytes) -> None:
        self.wfile.write(f"{len(payload):X}\r\n".encode("ascii"))
        self.wfile.write(payload)
        self.wfile.write(b"\r\n")

    def _normalized_path(self) -> str:
        return urlsplit(self.path).path.rstrip("/") or "/"

    @staticmethod
    def _json_response(
        status: int, payload: dict[str, Any]
    ) -> ProtocolResponse:
        body = json.dumps(
            payload, ensure_ascii=False, separators=(",", ":")
        ).encode("utf-8")
        return ProtocolResponse(
            status=status,
            media_type="application/json; charset=utf-8",
            body=body,
        )
