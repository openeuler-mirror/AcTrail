from __future__ import annotations

import math
from dataclasses import dataclass

from server_core.connection.http.config import HTTPConfig
from server_core.connection.https.config import HTTPSConfig


@dataclass(frozen=True, slots=True)
class ServerCoreConfig:
    http: HTTPConfig
    https: HTTPSConfig | None
    max_request_bytes: int
    request_timeout_seconds: float
    api_key: str | None
    log_requests: bool

    def __post_init__(self) -> None:
        if (
            self.https is not None
            and self.http.bind_host == self.https.bind_host
            and self.http.bind_port != 0
            and self.http.bind_port == self.https.bind_port
        ):
            raise ValueError(
                "HTTP and HTTPS cannot bind the same host and port"
            )
        if self.max_request_bytes <= 0:
            raise ValueError("max_request_bytes must be positive")
        if (
            not math.isfinite(self.request_timeout_seconds)
            or self.request_timeout_seconds <= 0
        ):
            raise ValueError(
                "request_timeout_seconds must be finite and positive"
            )
