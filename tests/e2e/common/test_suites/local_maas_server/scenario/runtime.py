from __future__ import annotations

from dataclasses import replace
from threading import Lock
from .model import (
    ResponseTemplate,
    ScenarioDefinition,
    ScenarioEmission,
    ScenarioRequest,
    ScenarioRuntimeError,
    UsageSnapshot,
)
from .scenario_generator.interface import (
    GeneratorExecution,
    ScenarioGenerationError,
)
from .tool_alias import ToolAliasConversionError, ToolAliasConverter


class ScenarioRuntime:
    def __init__(
        self,
        definition: ScenarioDefinition,
        tool_aliases: ToolAliasConverter,
    ):
        generator = definition.generator.canonicalize(tool_aliases)
        if generator is not definition.generator:
            definition = replace(definition, generator=generator)
        self.definition = definition
        self._tool_aliases = tool_aliases
        self._lock = Lock()
        self._execution: GeneratorExecution
        self._pending: ResponseTemplate | None = None
        self._response_index = 0
        self._reset()

    def reset(self) -> None:
        with self._lock:
            self._reset()

    def reserve(self, request: ScenarioRequest) -> ScenarioEmission:
        with self._lock:
            template = self._pending
            if template is None:
                try:
                    template = self._execution.next(
                        self._tool_aliases.generation_options(request)
                    )
                except ScenarioGenerationError as error:
                    raise ScenarioRuntimeError(
                        "scenario_mismatch",
                        f"scenario {self.definition.scenario_id!r} has no "
                        f"compatible response: {error}",
                    ) from error
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

            try:
                template = self._tool_aliases.convert(template, request)
            except ToolAliasConversionError as error:
                self._pending = template
                raise ScenarioRuntimeError(
                    "scenario_mismatch",
                    f"template {template.source_path} did not match the "
                    f"request: {error}",
                ) from error

            self._pending = None
            usage_delta = template.response.usage_delta
            emission = ScenarioEmission(
                index=self._response_index,
                template=template,
                usage=UsageSnapshot(
                    input_tokens=request.input_tokens,
                    output_tokens=usage_delta.output_tokens,
                ),
            )
            self._response_index += 1
            return emission

    def _reset(self) -> None:
        self._execution = self.definition.generator.reset()
        self._pending = None
        self._response_index = 0
