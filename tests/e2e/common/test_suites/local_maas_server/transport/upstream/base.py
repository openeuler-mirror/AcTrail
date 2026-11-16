from __future__ import annotations

import http.client
from abc import ABC, abstractmethod
from dataclasses import dataclass
from typing import Iterator
from urllib.parse import SplitResult, urlsplit


@dataclass(frozen=True, slots=True)
class UpstreamConfig:
    base_url: str
    api_key: str
    model: str | None = None

    def __post_init__(self) -> None:
        if not self.api_key:
            raise ValueError("upstream api_key must be non-empty")
        parsed = urlsplit(self.base_url)
        if parsed.scheme not in {"http", "https"} or not parsed.netloc:
            raise ValueError(
                "upstream base_url must be an absolute http(s) URL"
            )


class UpstreamResponse:
    def __init__(self, status: int, media_type: str):
        self._status = status
        self._media_type = media_type

    @property
    def status(self) -> int:
        return self._status

    @property
    def media_type(self) -> str:
        return self._media_type

    @property
    def body(self) -> bytes | None:
        return None

    @property
    def stream(self) -> UpstreamStream | None:
        return None

    @property
    def is_error(self) -> bool:
        return self.status >= 400


class DirectUpstreamResponse(UpstreamResponse):
    def __init__(self, status: int, media_type: str, payload: bytes):
        super().__init__(status, media_type)
        self._payload = payload

    @property
    def body(self) -> bytes:
        return self._payload


class UpstreamStream:
    def __init__(
        self,
        response: http.client.HTTPResponse,
        connection: http.client.HTTPConnection,
    ):
        self._response = response
        self._connection = connection
        self._closed = False

    @property
    def status(self) -> int:
        return self._response.status

    def lines(self) -> Iterator[bytes]:
        try:
            while True:
                line = self._response.readline()
                if not line:
                    break
                yield line
        finally:
            self.close()

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        self._connection.close()


class StreamingUpstreamResponse(UpstreamResponse):
    def __init__(
        self,
        status: int,
        media_type: str,
        stream: UpstreamStream,
    ):
        super().__init__(status, media_type)
        self._stream = stream

    @property
    def stream(self) -> UpstreamStream:
        return self._stream


class UpstreamClient(ABC):
    def __init__(self, request_timeout_seconds: float):
        self._timeout = request_timeout_seconds

    def forward(
        self,
        config: UpstreamConfig,
        path: str,
        document: dict[str, object],
        *,
        stream: bool,
    ) -> UpstreamResponse:
        parsed = urlsplit(config.base_url)
        request_path = parsed.path.rstrip("/") + path
        connection = self._connect(parsed)
        try:
            self._request(
                connection,
                config,
                request_path,
                document,
                stream=stream,
            )
            response = connection.getresponse()
            media_type = response.getheader(
                "Content-Type",
                (
                    "text/event-stream; charset=utf-8"
                    if stream
                    else "application/json; charset=utf-8"
                ),
            )
            if not stream or response.status >= 400:
                payload = response.read()
                connection.close()
                return DirectUpstreamResponse(
                    status=response.status,
                    media_type=media_type,
                    payload=payload,
                )
            return StreamingUpstreamResponse(
                status=response.status,
                media_type=media_type,
                stream=UpstreamStream(response, connection),
            )
        except Exception:
            connection.close()
            raise

    def _connect(
        self, parsed: SplitResult
    ) -> http.client.HTTPConnection:
        host = parsed.hostname or ""
        port = parsed.port
        if parsed.scheme == "https":
            return http.client.HTTPSConnection(
                host, port, timeout=self._timeout
            )
        return http.client.HTTPConnection(host, port, timeout=self._timeout)

    @abstractmethod
    def _request(
        self,
        connection: http.client.HTTPConnection,
        config: UpstreamConfig,
        request_path: str,
        document: dict[str, object],
        *,
        stream: bool,
    ) -> None:
        raise NotImplementedError
