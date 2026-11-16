from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Mapping

from ..model import ResponseTemplate, ToolCall, ToolDefinition
from .schema import ToolField, ToolSchema, ToolSchemaRegistry


_DEFAULT_SCHEMAS = (
    ToolSchema(
        name="bash",
        aliases=frozenset({"exec", "shell", "run_command", "terminal"}),
        fields=(ToolField(key="command", aliases=frozenset({"cmd", "script"})),),
    ),
    ToolSchema(
        name="read",
        aliases=frozenset({"read_file", "file_read"}),
        fields=(ToolField(key="path", aliases=frozenset({"file_path"})),),
    ),
    ToolSchema(
        name="write",
        aliases=frozenset({"write_file", "file_write"}),
        fields=(
            ToolField(key="path", aliases=frozenset({"file_path"})),
            ToolField(key="content", aliases=frozenset({"text"})),
        ),
    ),
    ToolSchema(
        name="grep",
        aliases=frozenset({"search", "search_text", "search_files"}),
        fields=(
            ToolField(key="pattern", aliases=frozenset({"query"})),
            ToolField(key="path", aliases=frozenset({"directory"})),
        ),
    ),
    ToolSchema(
        name="glob",
        aliases=frozenset({"find_files"}),
        fields=(
            ToolField(key="pattern", aliases=frozenset({"glob"})),
            ToolField(key="path", aliases=frozenset({"directory"})),
        ),
    ),
)


@dataclass(frozen=True, slots=True)
class ToolAliasConfig:
    """Declarative canonical tool schemas plus the runtime registry facade.

    The registry is derived once and mutates only to register unknown tools
    (strict-match, no aliases), so future agents only need a ToolSchema entry
    (or nothing at all for fully unknown tools).
    """

    schemas: tuple[ToolSchema, ...] = _DEFAULT_SCHEMAS
    _registry: ToolSchemaRegistry = field(init=False, repr=False, compare=False)

    def __post_init__(self) -> None:
        object.__setattr__(
            self,
            "_registry",
            ToolSchemaRegistry(self.schemas),
        )

    @property
    def registry(self) -> ToolSchemaRegistry:
        return self._registry

    @classmethod
    def default(cls) -> ToolAliasConfig:
        return cls()

    def canonical_name(self, client_name: str) -> str:
        return self.registry.canonicalize_call(
            ToolCall(name=client_name, arguments={})
        ).name

    def canonical_arguments(
        self,
        canonical_tool_name: str,
        arguments: Mapping[str, Any],
    ) -> dict[str, Any]:
        return self.registry.canonicalize_call(
            ToolCall(
                name=canonical_tool_name,
                arguments=dict(arguments),
            )
        ).arguments

    def canonicalize_call(self, call: ToolCall) -> ToolCall:
        return self.registry.canonicalize_call(call)

    def canonicalize_template(
        self,
        template: ResponseTemplate,
    ) -> ResponseTemplate:
        return self.registry.canonicalize_template(template)

    def convert_call(
        self,
        call: ToolCall,
        tools: tuple[ToolDefinition, ...],
    ) -> ToolCall | None:
        return self.registry.convert_call(call, tools)
