from __future__ import annotations

from dataclasses import dataclass
from types import MappingProxyType
from typing import Mapping


@dataclass(frozen=True, slots=True)
class ToolAliasConfig:
    tool_aliases: Mapping[str, tuple[str, ...]]
    argument_aliases: Mapping[str, Mapping[str, str]]

    @classmethod
    def default(cls) -> ToolAliasConfig:
        return cls(
            tool_aliases=MappingProxyType(
                {
                    "exec": ("bash",),
                    "shell": ("bash",),
                    "run_command": ("bash",),
                    "terminal": ("bash",),
                    "read_file": ("read",),
                    "file_read": ("read",),
                    "write_file": ("write",),
                    "file_write": ("write",),
                    "search": ("grep",),
                    "search_text": ("grep",),
                    "search_files": ("grep",),
                    "find_files": ("glob",),
                }
            ),
            argument_aliases=MappingProxyType(
                {
                    "bash": MappingProxyType(
                        {
                            "cmd": "command",
                            "script": "command",
                        }
                    ),
                    "read": MappingProxyType({"file_path": "path"}),
                    "write": MappingProxyType(
                        {
                            "file_path": "path",
                            "text": "content",
                        }
                    ),
                    "grep": MappingProxyType(
                        {
                            "query": "pattern",
                            "directory": "path",
                        }
                    ),
                    "glob": MappingProxyType(
                        {
                            "glob": "pattern",
                            "directory": "path",
                        }
                    ),
                }
            ),
        )
