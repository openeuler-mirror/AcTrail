from __future__ import annotations

from dataclasses import dataclass
from typing import Iterator

from ...model import ResponseTemplate, ScenarioConfigurationError
from ..interface import (
    GenerationOptions,
    GenerationUnavailable,
    GeneratorParameters,
    ScenarioGenerator,
)


@dataclass(frozen=True, slots=True)
class LoopGenerator(ScenarioGenerator):
    generator: ScenarioGenerator
    count: int | None

    def __post_init__(self) -> None:
        if self.generator.is_infinite:
            raise ScenarioConfigurationError(
                "loop child must be finite so each iteration can finish"
            )
        if self.count is not None and (
            isinstance(self.count, bool)
            or not isinstance(self.count, int)
            or self.count <= 0
        ):
            raise ScenarioConfigurationError(
                "loop count must be a positive integer when present"
            )

    @property
    def kind(self) -> str:
        return "loop"

    @property
    def is_infinite(self) -> bool:
        return self.count is None

    def generate(
        self, parameters: GeneratorParameters
    ) -> Iterator[ResponseTemplate | GenerationUnavailable]:
        iteration = 0
        while self.count is None or iteration < self.count:
            yield from self.generator.generate(parameters)
            iteration += 1

    def is_eligible(self, options: GenerationOptions) -> bool:
        return self.generator.is_eligible(options)
