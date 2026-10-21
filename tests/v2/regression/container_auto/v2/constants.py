"""Runtime defaults for this regression case. Durations are seconds."""

from dataclasses import dataclass


@dataclass(frozen=True)
class ContainerAutoConstants:
    start_daemon_timeout_seconds: int = 30
    require_matrix_evidence_timeout_seconds: int = 15
    wait_for_trace_after_timeout_seconds: int = 15
    host_command_timeout_seconds: int = 60
    start_daemon_poll_seconds: float = 0.1
    require_matrix_evidence_poll_seconds: float = 0.2
    wait_exec_timeout_seconds: int = 180
    wait_for_trace_after_poll_seconds: float = 0.1
    process_stop_timeout_seconds: int = 3
    daemon_stop_timeout_seconds: int = 5
    target_url: str = 'https://example.com/'
    new_container_ready_timeout_seconds: int = 30
    deployment_contract_problem_timeout_seconds: int = 60
    docker_info_timeout_seconds: int = 15
    timeout_seconds: int = 1200
    cleanup_grace_seconds: int = 30
    base_image: str = 'openeuler/openeuler:24.03-lts-sp3'
    rebuild_image: bool = False

    event_collector_mask: int = 0x0F
    event_collector_ebpf: int = 1
    event_collector_process_seccomp: int = 2
    injection_socket_timeout_seconds: float = 2.0
    injection_audit_timeout_seconds: float = 10.0
    injection_audit_poll_seconds: float = 0.2
    injection_receive_bytes: int = 4096
    tls_frame_header_format: str = "<2sBBI"
    tls_frame_magic: bytes = b"AT"
    tls_frame_version: int = 1
    tls_payload_frame_kind: int = 1

CASE_CONSTANTS = ContainerAutoConstants()
