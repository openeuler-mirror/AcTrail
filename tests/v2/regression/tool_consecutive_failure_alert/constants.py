"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ToolConsecutiveFailureAlertConstants:
    plugin_instance: str = 'tool-alert.v2'
    installed_plugin_instance: str = 'tool-alert.installed'
    marker_random_bytes: int = 6
    launch_diagnostic_tail_characters: int = 4000
    brief_diagnostic_tail_characters: int = 1000
    command_diagnostic_tail_characters: int = 2000


CASE_CONSTANTS = ToolConsecutiveFailureAlertConstants()
