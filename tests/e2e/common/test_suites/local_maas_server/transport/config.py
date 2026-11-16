from __future__ import annotations

from dataclasses import dataclass

from .upstream import UpstreamConfig


@dataclass(frozen=True, slots=True)
class TransportConfig:
    upstream: UpstreamConfig
