from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from .scenario_generator.interface import ScenarioGenerator


class ScenarioConfigurationError(RuntimeError):
    """Raised when a scenario cannot be constructed safely at startup."""


class ScenarioRuntimeError(RuntimeError):
    """A request-local scenario failure."""

    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True, slots=True)
class ScenarioRequest:
    protocol: str
    stream: bool
    model: str
    include_usage: bool
    input_tokens: int
    tools: tuple[ToolDefinition, ...]


@dataclass(frozen=True, slots=True)
class ToolDefinition:
    name: str
    input_schema: dict[str, Any]


@dataclass(frozen=True, slots=True)
class RequestExpectation:
    protocol: str | None
    stream: bool | None
    model: str | None

    def mismatch(self, request: ScenarioRequest) -> str | None:
        if self.protocol is not None and self.protocol != request.protocol:
            return f"protocol must be {self.protocol}, got {request.protocol}"
        if self.stream is not None and self.stream != request.stream:
            return f"stream must be {self.stream}, got {request.stream}"
        if self.model is not None and self.model != request.model:
            return f"model must be {self.model!r}, got {request.model!r}"
        return None


@dataclass(frozen=True, slots=True)
class UsageDelta:
    output_tokens: int


@dataclass(frozen=True, slots=True)
class UsageSnapshot:
    input_tokens: int
    output_tokens: int


@dataclass(frozen=True, slots=True)
class ToolCall:
    name: str
    arguments: dict[str, Any]


@dataclass(frozen=True, slots=True)
class ResponseBlock:
    kind: str
    fragments: tuple[str, ...]
    tool_call: ToolCall | None

    @property
    def text(self) -> str:
        return "".join(self.fragments)


@dataclass(frozen=True, slots=True)
class ResponseSpec:
    model: str | None
    blocks: tuple[ResponseBlock, ...]
    stop: str
    usage_delta: UsageDelta


@dataclass(frozen=True, slots=True)
class ResponseTemplate:
    source_path: str
    expectation: RequestExpectation
    response: ResponseSpec


@dataclass(frozen=True, slots=True)
class ScenarioDefinition:
    scenario_id: str
    description: str
    generator: ScenarioGenerator
    source: Path


@dataclass(frozen=True, slots=True)
class ScenarioSummary:
    scenario_id: str
    description: str


@dataclass(frozen=True, slots=True)
class ScenarioEmission:
    index: int
    template: ResponseTemplate
    usage: UsageSnapshot
