from __future__ import annotations

from threading import Lock
from typing import Iterator

from .model import (
    ResponseTemplate,
    ScenarioDefinition,
    ScenarioEmission,
    ScenarioRequest,
    ScenarioRuntimeError,
    UsageSnapshot,
)
from .scenario_generator.interface import GeneratorParameters


class ScenarioRuntime:
    def __init__(self, definition: ScenarioDefinition):
        self.definition = definition
        self._iterator: Iterator[ResponseTemplate] = iter(
            definition.generator.generate(GeneratorParameters())
        )
        self._lock = Lock()
        self._pending: ResponseTemplate | None = None
        self._response_index = 0
        self._cumulative_input_tokens = 0

    def reserve(self, request: ScenarioRequest) -> ScenarioEmission:
        with self._lock:
            template = self._pending
            if template is None:
                try:
                    template = next(self._iterator)
                except StopIteration as error:
                    raise ScenarioRuntimeError(
                        "scenario_exhausted",
                        f"scenario {self.definition.scenario_id!r} exhausted "
                        f"after {self._response_index} responses",
                    ) from error

            mismatch = template.expectation.mismatch(request)
            if mismatch is not None:
                self._pending = template
                raise ScenarioRuntimeError(
                    "scenario_mismatch",
                    f"template {template.source_path} did not match the "
                    f"request: {mismatch}",
                )

            self._pending = None
            usage_delta = template.response.usage_delta
            self._cumulative_input_tokens += usage_delta.input_tokens
            emission = ScenarioEmission(
                index=self._response_index,
                template=template,
                usage=UsageSnapshot(
                    input_tokens=self._cumulative_input_tokens,
                    output_tokens=usage_delta.output_tokens,
                ),
            )
            self._response_index += 1
            return emission
