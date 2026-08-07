from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from typing import Iterator, Protocol

from ..model import ResponseTemplate, ToolCall


class ToolCallCandidateFilter(Protocol):
    def accepts(self, call: ToolCall) -> bool: ...


@dataclass(frozen=True, slots=True)
class GenerationOptions:
    tool_calls: ToolCallCandidateFilter


@dataclass(frozen=True, slots=True)
class GenerationUnavailable:
    reason: str


class ScenarioGenerationError(RuntimeError):
    pass


@dataclass(slots=True)
class GeneratorParameters:
    _random_invocations: dict[str, int] = field(default_factory=dict)
    options: GenerationOptions | None = None

    def next_random_invocation(self, node_path: str) -> int:
        invocation = self._random_invocations.get(node_path, 0)
        self._random_invocations[node_path] = invocation + 1
        return invocation


class GeneratorExecution:
    def __init__(self, generator: ScenarioGenerator):
        self._parameters = GeneratorParameters()
        self._iterator = generator.generate(self._parameters)

    def next(self, options: GenerationOptions) -> ResponseTemplate:
        self._parameters.options = options
        result = next(self._iterator)
        if isinstance(result, GenerationUnavailable):
            raise ScenarioGenerationError(result.reason)
        return result


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
    ) -> Iterator[ResponseTemplate | GenerationUnavailable]:
        """Create the lazy response iterator for one server execution."""
        raise NotImplementedError

    @abstractmethod
    def is_eligible(self, options: GenerationOptions) -> bool:
        raise NotImplementedError

    def reset(self) -> GeneratorExecution:
        return GeneratorExecution(self)
