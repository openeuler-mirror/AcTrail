from __future__ import annotations

import hashlib
import random
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
class RandomGenerator(ScenarioGenerator):
    generators: tuple[ScenarioGenerator, ...]
    count: int | None
    seed: int
    node_path: str

    def __post_init__(self) -> None:
        if not self.generators:
            raise ScenarioConfigurationError(
                "random generator must contain at least one generator"
            )
        if any(generator.is_infinite for generator in self.generators):
            raise ScenarioConfigurationError(
                "random generator children must be finite"
            )
        if self.count is not None and (
            isinstance(self.count, bool)
            or not isinstance(self.count, int)
            or self.count <= 0
        ):
            raise ScenarioConfigurationError(
                "random count must be a positive integer when present"
            )

    @property
    def kind(self) -> str:
        return "random"

    @property
    def is_infinite(self) -> bool:
        return self.count is None

    def generate(
        self, parameters: GeneratorParameters
    ) -> Iterator[ResponseTemplate | GenerationUnavailable]:
        invocation = parameters.next_random_invocation(self.node_path)
        seed_material = (
            f"{self.node_path}\0{invocation}".encode("utf-8")
        )
        seed_digest = hashlib.sha256(seed_material).digest()
        effective_seed = (
            int.from_bytes(seed_digest[:8], "big") ^ self.seed
        )
        chooser = random.Random(effective_seed)
        iteration = 0
        while self.count is None or iteration < self.count:
            options = parameters.options
            eligible = (
                ()
                if options is None
                else tuple(
                    generator
                    for generator in self.generators
                    if generator.is_eligible(options)
                )
            )
            if not eligible:
                yield GenerationUnavailable(
                    f"random generator {self.node_path} has no compatible "
                    "candidate"
                )
                continue
            generator = chooser.choice(eligible)
            yield from generator.generate(parameters)
            iteration += 1

    def is_eligible(self, options: GenerationOptions) -> bool:
        return any(
            generator.is_eligible(options)
            for generator in self.generators
        )
