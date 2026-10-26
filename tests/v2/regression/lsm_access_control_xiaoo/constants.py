"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class LsmAccessControlXiaooConstants:
    require_workload_directories: bool = False
    ready_timeout_seconds: int = 15
    peer_poll_seconds: float = 0.1
    policy_max_entries: int = 64
    write_patch_path_max_bytes: int = 0
    write_patch_audit_buffer_bytes: int = 0
    write_patch_audit_batch_max: int = 0
    diagnostic_bytes: int = 65536
    denial_repetitions: int = 64
    denial_pause: float = 0.01
    require_rejected_domain: str = 'command'
    init_web_port: int = 0
    publish_timeout_ms: int = 1000
    publish_concurrency_limit: int = 1
    init_ipv6: bool = False
    bind_port: int = 0
    local_stream_reasoning_tokens: int = 1
    workload_denial_repetitions: int = 1
    workload_denial_pause: int = 0
    launch_diagnostic_tail_characters: int = 4000
    priority: int = 0
    specific_rule_priority: int = 20
    ipv6: bool = False
    llm_provider: str = 'deepseek'
    llm_model: str = 'deepseek-chat'
    llm_max_tokens: int = 128
    llm_context_window: int = 32768
    llm_reasoning_effort: str = 'off'
    agent_max_turns: int = 3
    audit_enabled: bool = True
    alert_window_ms: int = 60000
    alert_max_groups: int = 256
    deny_on_error: bool = True
    periodic_alert_window_ms: int = 200
    capacity_alert_max_groups: int = 1
    policy_capacity_minimum: int = 32
    policy_capacity_maximum: int = 128
    no_proxy_hosts: str = '127.0.0.1,localhost'
    loopback_host: str = '127.0.0.1'
    ipv6_loopback_host: str = '::1'
    witness_host: str = '127.0.0.2'


CASE_CONSTANTS = LsmAccessControlXiaooConstants()
