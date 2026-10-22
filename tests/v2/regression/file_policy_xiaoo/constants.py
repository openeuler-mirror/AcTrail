"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class FilePolicyXiaooConstants:
    ready_timeout_seconds: int = 15
    launch_diagnostic_tail_characters: int = 4000
    llm_provider: str = 'deepseek'
    llm_model: str = 'deepseek-chat'
    llm_max_tokens: int = 128
    llm_context_window: int = 32768
    llm_reasoning_effort: str = 'off'
    agent_max_turns: int = 3
    no_proxy_hosts: str = '127.0.0.1,localhost'


CASE_CONSTANTS = FilePolicyXiaooConstants()
