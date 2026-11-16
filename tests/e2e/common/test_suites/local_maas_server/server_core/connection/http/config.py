from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class HTTPConfig:
    bind_host: str
    bind_port: int

    def __post_init__(self) -> None:
        if not self.bind_host:
            raise ValueError("HTTP bind_host must be non-empty")
        if not 0 <= self.bind_port <= 65535:
            raise ValueError("HTTP bind_port must be between 0 and 65535")
