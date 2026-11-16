"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class SemanticActionBoundariesConstants:
    plugin_package: str = 'otel-jsonl'
    plugin_instance: str = 'v2.semantic-action-boundaries'
    web_host: str = '127.0.0.1'
    launch_command_host_ebpf: str = 'auto'
    launch_command_seccomp_notify: str = 'auto'
    marker_random_bytes: int = 6
    launch_diagnostic_tail_characters: int = 4000
    command_diagnostic_tail_characters: int = 2000


CASE_CONSTANTS = SemanticActionBoundariesConstants()
