"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ToolFrequentFailureAlertConstants:
    plugin_instance: str = 'tool-frequent-alert.v2'
    installed_plugin_instance: str = 'tool-frequent-alert.installed'
    run_positive_round_successful_paths: tuple = ()
    assert_payload_require_tool_name: bool = True
    assert_payload_strict_values: bool = True
    assert_payload_expected_total_count: int = 3
    assert_payload_expected_failure_rate: float = 1.0
    marker_random_bytes: int = 6
    launch_diagnostic_tail_characters: int = 4000
    brief_diagnostic_tail_characters: int = 1000
    command_diagnostic_tail_characters: int = 2000


CASE_CONSTANTS = ToolFrequentFailureAlertConstants()
