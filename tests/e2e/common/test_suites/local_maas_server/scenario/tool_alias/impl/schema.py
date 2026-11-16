from __future__ import annotations

from dataclasses import replace
from typing import Any

from ...model import ResponseTemplate, ScenarioRequest, ToolCall, ToolDefinition
from ...scenario_generator.interface import GenerationOptions
from ..config import ToolAliasConfig
from ..interface import ToolAliasConversionError, ToolAliasConverter


class RuntimeToolAliasResolver:
    """Resolve a (possibly raw) tool call into the client's own schema."""

    def __init__(
        self,
        config: ToolAliasConfig,
        tools: tuple[ToolDefinition, ...],
    ):
        self._config = config
        self._tools = tools

    def resolve(self, call: ToolCall) -> ToolCall | None:
        return self._config.convert_call(call, self._tools)


class SchemaToolCallCandidateFilter:
    def __init__(
        self,
        resolver: RuntimeToolAliasResolver,
    ):
        self._resolver = resolver

    def accepts(self, call: ToolCall) -> bool:
        return self._resolver.resolve(call) is not None


class SchemaToolAliasConverter(ToolAliasConverter):
    def __init__(self, config: ToolAliasConfig):
        self._config = config

    def generation_options(
        self,
        request: ScenarioRequest,
    ) -> GenerationOptions:
        return GenerationOptions(
            tool_calls=SchemaToolCallCandidateFilter(
                RuntimeToolAliasResolver(self._config, request.tools),
            ),
            declares_tools=bool(request.tools),
        )

    def canonicalize_template(
        self,
        template: ResponseTemplate,
    ) -> ResponseTemplate:
        return self._config.canonicalize_template(template)

    def convert(
        self,
        template: ResponseTemplate,
        request: ScenarioRequest,
    ) -> ResponseTemplate:
        canonical = self.canonicalize_template(template)
        resolver = RuntimeToolAliasResolver(self._config, request.tools)
        converted = tuple(
            self._convert_block(block, resolver, request.tools)
            for block in canonical.response.blocks
        )
        if converted == canonical.response.blocks:
            return canonical
        return replace(
            canonical,
            response=replace(canonical.response, blocks=converted),
        )

    @staticmethod
    def _convert_block(
        block: Any,
        resolver: RuntimeToolAliasResolver,
        tools: tuple[ToolDefinition, ...],
    ) -> Any:
        call = block.tool_call
        if call is None:
            return block
        converted = resolver.resolve(call)
        if converted is not None:
            return replace(block, tool_call=converted)
        offered = ", ".join(tool.name for tool in tools) or "none"
        raise ToolAliasConversionError(
            f"tool {call.name!r} has no compatible alias; "
            f"request offered: {offered}"
        )
