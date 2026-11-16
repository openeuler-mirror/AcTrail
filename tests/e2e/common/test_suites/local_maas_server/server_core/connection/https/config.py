from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True, slots=True)
class HTTPSConfig:
    bind_host: str
    bind_port: int
    best_effort: bool
    certificate_work_dir: Path | None
    openssl_binary: str
    certificate_validity_days: int

    def __post_init__(self) -> None:
        if not self.bind_host:
            raise ValueError("HTTPS bind_host must be non-empty")
        if not 0 <= self.bind_port <= 65535:
            raise ValueError("HTTPS bind_port must be between 0 and 65535")
        if not self.openssl_binary:
            raise ValueError("HTTPS openssl_binary must be non-empty")
        if self.certificate_validity_days <= 0:
            raise ValueError(
                "HTTPS certificate_validity_days must be positive"
            )
