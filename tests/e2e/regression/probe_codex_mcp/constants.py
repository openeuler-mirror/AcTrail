"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ProbeCodexMcpConstants:
    reasoning_effort: str = 'low'
    local_server_name: str = 'actrail_codex_stdio'
    tool_name: str = 'emit_marker'
    artifact_root: str = 'temp/v2-regression/mcp'
    tool_description_padding_bytes: int = 8192
    marker_random_bytes: int = 6
    launch_diagnostic_tail_characters: int = 4000
    fallback_model: str = 'gpt-5.5'


CASE_CONSTANTS = ProbeCodexMcpConstants()
