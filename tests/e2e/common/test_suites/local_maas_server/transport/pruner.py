from __future__ import annotations

from typing import Any


class ToolPruner:
    """Filter outbound MaaS requests down to a per-session tool whitelist."""

    def __init__(self, whitelist: frozenset[str]):
        self._whitelist = whitelist

    def prune(self, document: dict[str, Any]) -> dict[str, Any]:
        result = dict(document)
        raw_tools = document.get("tools")
        if isinstance(raw_tools, list):
            result["tools"] = [
                tool
                for tool in raw_tools
                if self._openai_tool_kept(tool)
            ]
        self._prune_openai_tool_choice(result)
        return result

    def _prune_openai_tool_choice(self, document: dict[str, Any]) -> None:
        choice = document.get("tool_choice")
        if not isinstance(choice, dict):
            return
        function = choice.get("function")
        if not isinstance(function, dict):
            return
        name = function.get("name")
        if not isinstance(name, str) or not self._kept(name):
            document.pop("tool_choice", None)

    def _openai_tool_kept(self, tool: object) -> bool:
        if not isinstance(tool, dict):
            return False
        function = tool.get("function")
        if not isinstance(function, dict):
            return False
        name = function.get("name")
        return isinstance(name, str) and self._kept(name)

    def _kept(self, name: str) -> bool:
        return name.casefold() in self._whitelist
