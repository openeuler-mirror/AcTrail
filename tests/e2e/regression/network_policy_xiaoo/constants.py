"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class NetworkPolicyXiaooConstants:
    ready_timeout_seconds: int = 15
    evidence_timeout_seconds: int = 15
    web_host: str = '127.0.0.1'
    actrail_plugin_dir: str = '~/.actrail/plugins'
    web_port: int = 0
    command_diagnostic_tail_characters: int = 2000
    llm_provider: str = 'deepseek'
    llm_model: str = 'deepseek-chat'
    llm_max_tokens: int = 128
    llm_context_window: int = 32768
    llm_reasoning_effort: str = 'off'
    no_proxy_hosts: str = '127.0.0.1,localhost'
    max_turns: int = 3


CASE_CONSTANTS = NetworkPolicyXiaooConstants()
