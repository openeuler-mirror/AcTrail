from __future__ import annotations

from .interface import ProtocolAdapter


class ProtocolRegistry:
    def __init__(self, adapters: tuple[ProtocolAdapter, ...]):
        if not adapters:
            raise ValueError("at least one protocol adapter is required")
        by_name: dict[str, ProtocolAdapter] = {}
        for adapter in adapters:
            if adapter.name in by_name:
                raise ValueError(f"duplicate protocol name: {adapter.name}")
            by_name[adapter.name] = adapter
        self._adapters = adapters
        self._by_name = by_name

    @property
    def adapters(self) -> tuple[ProtocolAdapter, ...]:
        return self._adapters

    @property
    def names(self) -> frozenset[str]:
        return frozenset(self._by_name)

    def by_name(self, name: str) -> ProtocolAdapter | None:
        return self._by_name.get(name)
