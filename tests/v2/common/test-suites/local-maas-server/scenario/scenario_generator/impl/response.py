from __future__ import annotations

from dataclasses import dataclass
from typing import Iterator

from ...model import ResponseTemplate
from ..interface import GeneratorParameters, ScenarioGenerator


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
    ) -> Iterator[ResponseTemplate]:
        del parameters
        yield self.template
