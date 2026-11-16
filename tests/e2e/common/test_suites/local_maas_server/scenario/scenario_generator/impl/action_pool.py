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
class ActionPoolGenerator(ScenarioGenerator):
    actions: tuple[ScenarioGenerator, ...]
    selection: str
    count: int | None
    seed: int
    node_path: str

    def __post_init__(self) -> None:
        if not self.actions:
            raise ScenarioConfigurationError(
                "action_pool generator must contain at least one action"
            )
        if any(action.is_infinite for action in self.actions):
            raise ScenarioConfigurationError(
                "action_pool actions must be finite"
            )
        if self.selection not in {"random", "sequential"}:
            raise ScenarioConfigurationError(
                "action_pool selection must be random or sequential"
            )
        if self.count is not None and (
            isinstance(self.count, bool)
            or not isinstance(self.count, int)
            or self.count <= 0
        ):
            raise ScenarioConfigurationError(
                "action_pool count must be a positive integer when present"
            )

    @property
    def kind(self) -> str:
        return "action_pool"

    @property
    def is_infinite(self) -> bool:
        return self.count is None

    def generate(
        self, parameters: GeneratorParameters
    ) -> Iterator[ResponseTemplate | GenerationUnavailable]:
        if self.selection == "sequential":
            yield from self._generate_sequential(parameters)
            return
        yield from self._generate_random(parameters)

    def _generate_sequential(
        self, parameters: GeneratorParameters
    ) -> Iterator[ResponseTemplate | GenerationUnavailable]:
        iteration = 0
        while self.count is None or iteration < self.count:
            eligible = self._eligible_actions(parameters)
            if not eligible:
                yield GenerationUnavailable(
                    f"action_pool {self.node_path} has no compatible action"
                )
                continue
            action = eligible[iteration % len(eligible)]
            yield from action.generate(parameters)
            iteration += 1

    def _generate_random(
        self, parameters: GeneratorParameters
    ) -> Iterator[ResponseTemplate | GenerationUnavailable]:
        invocation = parameters.next_random_invocation(self.node_path)
        seed_material = f"{self.node_path}\0{invocation}".encode("utf-8")
        seed_digest = hashlib.sha256(seed_material).digest()
        chooser = random.Random(
            int.from_bytes(seed_digest[:8], "big") ^ self.seed
        )
        iteration = 0
        while self.count is None or iteration < self.count:
            eligible = self._eligible_actions(parameters)
            if not eligible:
                yield GenerationUnavailable(
                    f"action_pool {self.node_path} has no compatible action"
                )
                continue
            yield from chooser.choice(eligible).generate(parameters)
            iteration += 1

    def is_eligible(self, options: GenerationOptions) -> bool:
        return any(action.is_eligible(options) for action in self.actions)

    def _eligible_actions(
        self,
        parameters: GeneratorParameters,
    ) -> tuple[ScenarioGenerator, ...]:
        options = parameters.options
        if options is None:
            return ()
        return tuple(
            action
            for action in self.actions
            if action.is_eligible(options)
        )
