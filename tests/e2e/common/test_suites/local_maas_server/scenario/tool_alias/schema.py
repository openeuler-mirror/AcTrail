"""Canonical tool schemas and the registry that drives both directions.

The registry is the single intermediate state: recording normalizes any
agent's tool call to the canonical schema (``canonicalize_call``), and replay
translates the canonical call back to the client's declared tools
(``convert_call``). Tools that were never declared get a strict-match schema
with no aliases, so unknown agents/tools keep working without new code.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from threading import RLock
from typing import Any, Iterable

from utils.naming import normalize_name

from ..model import ResponseTemplate, ToolCall, ToolDefinition


@dataclass(frozen=True, slots=True)
class ToolField:
    key: str
    aliases: frozenset[str] = frozenset()

    def matches(self, normalized: str) -> bool:
        if normalize_name(self.key) == normalized:
            return True
        return any(
            normalize_name(alias) == normalized
            for alias in self.aliases
        )


@dataclass(frozen=True, slots=True)
class ToolSchema:
    name: str
    aliases: frozenset[str] = frozenset()
    fields: tuple[ToolField, ...] = ()

    def matches_name(self, normalized: str) -> bool:
        if normalize_name(self.name) == normalized:
            return True
        return any(
            normalize_name(alias) == normalized
            for alias in self.aliases
        )

    def field_for(self, normalized: str) -> ToolField | None:
        for field in self.fields:
            if field.matches(normalized):
                return field
        return None


class ToolSchemaRegistry:
    """Name/field alias index with strict-match fallback for unknown tools."""

    def __init__(self, schemas: Iterable[ToolSchema] = ()):
        self._by_name: dict[str, ToolSchema] = {}
        self._lock = RLock()
        for schema in schemas:
            self._register(schema)

    def register(self, schema: ToolSchema) -> None:
        with self._lock:
            self._register(schema)

    def _register(self, schema: ToolSchema) -> None:
        self._by_name.setdefault(normalize_name(schema.name), schema)
        for alias in schema.aliases:
            self._by_name.setdefault(normalize_name(alias), schema)

    def ensure_tool(
        self,
        name: str,
        argument_keys: Iterable[str] = (),
    ) -> ToolSchema:
        """Resolve by name, or register a strict schema for an unknown tool."""

        normalized = normalize_name(name)
        schema = self._by_name.get(normalized)
        if schema is not None:
            return schema
        with self._lock:
            schema = self._by_name.get(normalized)
            if schema is not None:
                return schema
            schema = ToolSchema(
                name=name,
                aliases=frozenset(),
                fields=tuple(ToolField(key=key) for key in argument_keys),
            )
            self._register(schema)
            return schema

    def canonicalize_call(self, call: ToolCall) -> ToolCall:
        schema = self.ensure_tool(call.name, tuple(call.arguments))
        arguments: dict[str, Any] = {}
        for key, value in call.arguments.items():
            field = schema.field_for(normalize_name(key))
            arguments[field.key if field is not None else key] = value
        return ToolCall(name=schema.name, arguments=arguments)

    def canonicalize_template(
        self,
        template: ResponseTemplate,
    ) -> ResponseTemplate:
        blocks = tuple(
            self._canonicalize_block(block)
            for block in template.response.blocks
        )
        if blocks == template.response.blocks:
            return template
        return replace(
            template,
            response=replace(template.response, blocks=blocks),
        )

    def convert_call(
        self,
        call: ToolCall,
        client_tools: tuple[ToolDefinition, ...],
    ) -> ToolCall | None:
        canonical = self.canonicalize_call(call)
        schema = self._by_name.get(normalize_name(canonical.name))
        if schema is None:
            return None
        for tool in client_tools:
            if not schema.matches_name(normalize_name(tool.name)):
                continue
            arguments = self._convert_arguments(
                schema, tool, canonical.arguments
            )
            if arguments is not None:
                return ToolCall(name=tool.name, arguments=arguments)
        return None

    def _canonicalize_block(self, block: Any) -> Any:
        call = block.tool_call
        if call is None:
            return block
        return replace(block, tool_call=self.canonicalize_call(call))

    @staticmethod
    def _convert_arguments(
        schema: ToolSchema,
        tool: ToolDefinition,
        arguments: dict[str, Any],
    ) -> dict[str, Any] | None:
        raw_properties = tool.input_schema.get("properties", {})
        if not isinstance(raw_properties, dict):
            return None
        properties: dict[str, tuple[str, object]] = {}
        for actual_name, property_schema in raw_properties.items():
            normalized = normalize_name(actual_name)
            field = schema.field_for(normalized)
            identity = field.key if field is not None else normalized
            properties.setdefault(identity, (actual_name, property_schema))
        converted: dict[str, Any] = {}
        for argument_name, value in arguments.items():
            normalized = normalize_name(argument_name)
            field = schema.field_for(normalized)
            identity = field.key if field is not None else normalized
            entry = properties.get(identity)
            if entry is None:
                converted[argument_name] = value
                continue
            actual_name, property_schema = entry
            if not _value_matches_schema(value, property_schema):
                return None
            converted[actual_name] = value
        required = tool.input_schema.get("required", [])
        if not isinstance(required, list) or any(
            not isinstance(name, str) for name in required
        ):
            return None
        if not set(required).issubset(converted):
            return None
        return converted


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
