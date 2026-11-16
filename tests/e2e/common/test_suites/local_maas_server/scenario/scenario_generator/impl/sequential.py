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
class SequentialGenerator(ScenarioGenerator):
    generators: tuple[ScenarioGenerator, ...]

    def __post_init__(self) -> None:
        if not self.generators:
            raise ScenarioConfigurationError(
                "sequential generator must contain at least one generator"
            )
        if any(generator.is_infinite for generator in self.generators[:-1]):
            raise ScenarioConfigurationError(
                "an infinite generator must be the final sequential child"
            )

    @property
    def kind(self) -> str:
        return "sequential"

    @property
    def is_infinite(self) -> bool:
        return self.generators[-1].is_infinite

    def generate(
        self, parameters: GeneratorParameters
    ) -> Iterator[ResponseTemplate | GenerationUnavailable]:
        for generator in self.generators:
            yield from generator.generate(parameters)

    def is_eligible(self, options: GenerationOptions) -> bool:
        return all(
            generator.is_eligible(options)
            for generator in self.generators
        )
