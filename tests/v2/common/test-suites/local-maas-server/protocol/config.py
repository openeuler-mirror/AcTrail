from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class ProtocolConfig:
    default_model: str

    def __post_init__(self) -> None:
        if not self.default_model:
            raise ValueError("default_model must be non-empty")
