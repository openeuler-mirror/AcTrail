from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class McpProbeDefaults:
    tool_description_padding_bytes: int = 0


DEFAULTS = McpProbeDefaults()
