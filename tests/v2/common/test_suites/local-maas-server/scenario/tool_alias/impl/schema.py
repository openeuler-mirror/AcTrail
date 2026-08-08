from __future__ import annotations

from dataclasses import replace
from typing import Any

from ...model import (
    ResponseBlock,
    ResponseTemplate,
    ScenarioRequest,
    ToolCall,
    ToolDefinition,
)
from ...scenario_generator.interface import GenerationOptions
from ..config import ToolAliasConfig
from ..interface import ToolAliasConversionError, ToolAliasConverter


class SchemaToolCallCandidateFilter:
    def __init__(
        self,
        resolver: RuntimeToolAliasResolver,
    ):
        self._resolver = resolver

    def accepts(self, call: ToolCall) -> bool:
        return self._resolver.resolve(call) is not None


class RuntimeToolAliasResolver:
    def __init__(
        self,
        config: ToolAliasConfig,
        tools: tuple[ToolDefinition, ...],
    ):
        self._config = config
        self._tools_by_canonical_name = self._index_tools(tools)

    def resolve(self, call: ToolCall) -> ToolCall | None:
        canonical_name = call.name.casefold()
        for tool in self._tools_by_canonical_name.get(canonical_name, ()):
            arguments = self._convert_arguments(
                canonical_name,
                call,
                tool,
            )
            if arguments is not None:
                return ToolCall(
                    name=tool.name,
                    arguments=arguments,
                )
        return None

    def _index_tools(
        self,
        tools: tuple[ToolDefinition, ...],
    ) -> dict[str, tuple[ToolDefinition, ...]]:
        indexed: dict[str, list[ToolDefinition]] = {}
        for tool in tools:
            normalized_name = tool.name.casefold()
            canonical_names = self._config.tool_aliases.get(
                normalized_name,
                (normalized_name,),
            )
            for canonical_name in canonical_names:
                indexed.setdefault(canonical_name, []).append(tool)
        return {
            name: tuple(definitions)
            for name, definitions in indexed.items()
        }

    def _convert_arguments(
        self,
        canonical_tool_name: str,
        call: ToolCall,
        tool: ToolDefinition,
    ) -> dict[str, Any] | None:
        schema = tool.input_schema
        raw_properties = schema.get("properties", {})
        if not isinstance(raw_properties, dict):
            return None
        property_aliases = self._config.argument_aliases.get(
            canonical_tool_name,
            {},
        )
        properties: dict[str, tuple[str, object]] = {}
        for actual_name, property_schema in raw_properties.items():
            normalized_name = actual_name.casefold()
            canonical_name = property_aliases.get(
                normalized_name,
                normalized_name,
            )
            properties.setdefault(
                canonical_name,
                (actual_name, property_schema),
            )

        converted: dict[str, Any] = {}
        for argument_name, value in call.arguments.items():
            canonical_name = argument_name.casefold()
            property_entry = properties.get(canonical_name)
            if property_entry is None:
                if properties or schema.get("additionalProperties") is False:
                    return None
                actual_name = argument_name
                property_schema = None
            else:
                actual_name, property_schema = property_entry
            if not self._value_matches_schema(value, property_schema):
                return None
            converted[actual_name] = value

        required = schema.get("required", [])
        if not isinstance(required, list) or any(
            not isinstance(name, str) for name in required
        ):
            return None
        if not set(required).issubset(converted):
            return None
        return converted

    @staticmethod
    def _value_matches_schema(value: Any, schema: object) -> bool:
        if not isinstance(schema, dict):
            return True
        expected = schema.get("type")
        if expected is None:
            return True
        matches = {
            "string": lambda: isinstance(value, str),
            "integer": lambda: isinstance(value, int)
            and not isinstance(value, bool),
            "number": lambda: isinstance(value, (int, float))
            and not isinstance(value, bool),
            "boolean": lambda: isinstance(value, bool),
            "object": lambda: isinstance(value, dict),
            "array": lambda: isinstance(value, list),
            "null": lambda: value is None,
        }
        matcher = matches.get(expected)
        return matcher is not None and matcher()


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
            )
        )

    def convert(
        self,
        template: ResponseTemplate,
        request: ScenarioRequest,
    ) -> ResponseTemplate:
        resolver = RuntimeToolAliasResolver(self._config, request.tools)
        converted = tuple(
            self._convert_block(block, resolver, request.tools)
            for block in template.response.blocks
        )
        if converted == template.response.blocks:
            return template
        return replace(
            template,
            response=replace(template.response, blocks=converted),
        )

    def _convert_block(
        self,
        block: ResponseBlock,
        resolver: RuntimeToolAliasResolver,
        tools: tuple[ToolDefinition, ...],
    ) -> ResponseBlock:
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
