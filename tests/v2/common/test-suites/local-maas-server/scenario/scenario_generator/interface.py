from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from typing import Iterator

from ..model import ResponseTemplate


@dataclass(slots=True)
class GeneratorParameters:
    _random_invocations: dict[str, int] = field(default_factory=dict)

    def next_random_invocation(self, node_path: str) -> int:
        invocation = self._random_invocations.get(node_path, 0)
        self._random_invocations[node_path] = invocation + 1
        return invocation


class ScenarioGenerator(ABC):
    @property
    @abstractmethod
    def kind(self) -> str:
        raise NotImplementedError

    @property
    @abstractmethod
    def is_infinite(self) -> bool:
        raise NotImplementedError

    @abstractmethod
    def generate(
        self, parameters: GeneratorParameters
    ) -> Iterator[ResponseTemplate]:
        """Create the lazy response iterator for one server execution."""
        raise NotImplementedError
