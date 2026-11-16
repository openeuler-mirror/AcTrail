from __future__ import annotations

from dataclasses import dataclass
from typing import Iterator

from ...model import ResponseTemplate
from ..interface import (
    GenerationOptions,
    GenerationUnavailable,
    GeneratorParameters,
    ScenarioGenerator,
)


@dataclass(frozen=True, slots=True)
class ResponseGenerator(ScenarioGenerator):
    template: ResponseTemplate

    @property
    def kind(self) -> str:
        return "response"

    @property
    def is_infinite(self) -> bool:
        return False

    def generate(
        self, parameters: GeneratorParameters
    ) -> Iterator[ResponseTemplate | GenerationUnavailable]:
        while not self._is_currently_eligible(parameters):
            yield GenerationUnavailable(
                f"response {self.template.source_path} has no compatible "
                "tool candidate"
            )
        yield self.template

    def is_eligible(self, options: GenerationOptions) -> bool:
        return all(
            block.tool_call is None
            or options.tool_calls.accepts(block.tool_call)
            for block in self.template.response.blocks
        )

    def _is_currently_eligible(
        self,
        parameters: GeneratorParameters,
    ) -> bool:
        options = parameters.options
        return options is not None and self.is_eligible(options)
