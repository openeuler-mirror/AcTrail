"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class CommandPolicyXiaooConstants:
    ready_timeout_seconds: int = 15
    evidence_timeout_seconds: int = 15
    web_host: str = '127.0.0.1'
    bash: str = '/usr/bin/bash'
    bind_port: int = 0
    local_stream_reasoning_tokens: int = 1
    stop_timeout: int = 10
    priority: int = 20
    fallback_rule_priority: int = 10
    command_diagnostic_tail_characters: int = 2000
    launch_diagnostic_tail_characters: int = 4000
    llm_provider: str = 'deepseek'
    llm_model: str = 'deepseek-chat'
    llm_max_tokens: int = 128
    llm_context_window: int = 32768
    llm_reasoning_effort: str = 'off'
    no_proxy_hosts: str = '127.0.0.1,localhost'
    max_turns: int = 3
    loopback_host: str = '127.0.0.1'


CASE_CONSTANTS = CommandPolicyXiaooConstants()
