"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ProjectSubagentTrajectoryConstants:
    model: str = ''
    xiaoo_max_turns: int = 20
    plugin_package: str = 'otel-http'
    plugin_instance: str = 'v2.project-subagent-trajectory-otel-http'
    trace_random_bytes: int = 3
    web_host: str = '127.0.0.1'
    scenario_diagnostic_tail_characters: int = 6000
    command_diagnostic_tail_characters: int = 2000
    request_content_max_bytes: int = 16777216


CASE_CONSTANTS = ProjectSubagentTrajectoryConstants()
