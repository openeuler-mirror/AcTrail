"""Runtime settings for the Docker xiaoO regression case. Durations are seconds."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ContainerAgentXiaooConstants:
    image: str = "ubuntu:24.04"
    bin_dir: str = "target/release"
    xiaoo_bin: str = "/root/.cargo/bin/xiaoo"
    rebuild_image: bool = False

    case_timeout_seconds: int = 900
    case_cleanup_grace_seconds: int = 120
    docker_info_timeout_seconds: float = 15.0
    docker_ready_timeout_seconds: float = 30.0
    docker_stop_timeout_seconds: float = 10.0
    docker_kill_timeout_seconds: float = 5.0
    daemon_stop_timeout_seconds: float = 60.0
    daemon_kill_timeout_seconds: float = 30.0
    process_stop_timeout_seconds: float = 5.0
    process_kill_timeout_seconds: float = 5.0

    ready_timeout_seconds: float = 30.0
    launch_timeout_seconds: float = 180.0
    drain_timeout_seconds: float = 30.0
    viewer_timeout_seconds: float = 60.0
    container_start_stagger_seconds: float = 10.0
    active_overlap_seconds: float = 3.0
    start_stagger_tolerance_seconds: float = 0.5
    readiness_poll_seconds: float = 0.1
    process_child_poll_seconds: float = 0.02
    worker_poll_seconds: float = 0.05
    trace_poll_seconds: float = 0.05
    completion_poll_seconds: float = 0.1
    evidence_poll_seconds: float = 0.2
    process_wait_minimum_seconds: float = 0.1
    identity_process_hold_seconds: float = 30.0

    provider_host: str = "127.0.0.1"
    provider_port: int = 0
    provider_reasoning_tokens: int = 3
    provider_chunk_delay_seconds: float = 0.05
    llm_provider: str = "deepseek"
    llm_model: str = "deepseek-chat"
    llm_max_tokens: int = 128
    llm_context_window: int = 32768
    llm_reasoning_effort: str = "off"

    diagnostic_tail_lines: int = 80
    diagnostic_tail_characters: int = 16_000
    viewer_diagnostic_tail_characters: int = 2000


CASE_CONSTANTS = ContainerAgentXiaooConstants()
