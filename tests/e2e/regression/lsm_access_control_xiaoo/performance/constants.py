"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass, field


@dataclass(frozen=True)
class LsmAccessControlXiaooPerformanceConstants:
    report_input_filename: str = 'wasm-performance.json'
    report_output_filename: str = 'wasm-performance.md'
    static_report_filename: str = 'performance.json'
    run_lock_poll_seconds: int = 1
    bin_dir: str = 'target/release'
    policy_source: str = 'wasm'
    backends: list = field(default_factory=lambda: ['seccomp', 'lsm'])
    command_timeout: int = 60
    launch_timeout: int = 240
    lock_timeout: int = 60
    peer_poll_seconds: float = 0.05
    backlog: int = 4096
    count: int = 300
    rounds: int = 3
    warmup: int = 50
    timeout: int = 15
    stop_timeout: int = 2
    init_web_port: int = 0
    actrail_plugin_dir: str = '~/.actrail/plugins'
    launch_diagnostic_tail_characters: int = 4000
    keep_work: bool = False
    llm_provider: str = 'deepseek'
    llm_model: str = 'deepseek-chat'
    llm_max_tokens: int = 128
    llm_context_window: int = 32768
    llm_reasoning_effort: str = 'off'
    agent_max_turns: int = 3
    no_proxy_hosts: str = '127.0.0.1,localhost'
    loopback_host: str = '127.0.0.1'


CASE_CONSTANTS = LsmAccessControlXiaooPerformanceConstants()
