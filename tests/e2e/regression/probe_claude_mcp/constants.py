"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ProbeClaudeMcpConstants:
    local_server_name: str = 'actrail_claude_stdio'
    tool_name: str = 'emit_marker'
    artifact_root: str = 'temp/v2-regression/mcp'
    ready_timeout_seconds: int = 60
    ready_poll_interval_seconds: float = 0.05
    tool_description_padding_bytes: int = 8192
    marker_random_bytes: int = 6
    launch_diagnostic_tail_characters: int = 4000


CASE_CONSTANTS = ProbeClaudeMcpConstants()
