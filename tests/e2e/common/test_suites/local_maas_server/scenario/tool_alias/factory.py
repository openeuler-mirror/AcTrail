from __future__ import annotations

from .config import ToolAliasConfig
from .impl import SchemaToolAliasConverter
from .interface import ToolAliasConverter


class ToolAliasConverterFactory:
    def create(self, config: ToolAliasConfig) -> ToolAliasConverter:
        return SchemaToolAliasConverter(config)
