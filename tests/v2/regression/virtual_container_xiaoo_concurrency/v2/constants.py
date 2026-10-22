"""Runtime defaults for this regression case. Durations are seconds."""

from dataclasses import dataclass


@dataclass(frozen=True)
class VirtualContainerXiaooConcurrencyConstants:
    backend: str = 'stratovirt'
    image_pull_policy: str = 'never'
    ctr_runtime: str = 'io.containerd.kata.v2'
    runtime_timeout_seconds: int = 900
    ready_timeout_seconds: int = 90
    overlap_timeout_seconds: int = 30
    settle_seconds: int = 5
    workload_uid: int = 1000
    workload_gid: int = 39000
    image: str = 'docker.io/library/actrail-openeuler-workload:24.09'
    bind_host: str = '127.0.0.1'
    allow_port: int = 443
    proxy_relay_join_timeout_seconds: int = 1
    proxy_connect_timeout_seconds: int = 15
    start_opencode_egress_poll_seconds: float = 0.2
    validate_trace_poll_seconds: float = 0.5
    wait_for_paths_poll_seconds: float = 0.05
    workload_stop_timeout_seconds: int = 5
    start_opencode_egress_timeout_seconds: int = 1
    start_opencode_egress_grace_seconds: int = 1
    process_exit_timeout_seconds: int = 1
    service_stop_timeout_seconds: int = 2
    tcp_connect_timeout_seconds: float = 0.5
    wait_host_tcp_poll_seconds: float = 0.05
    wait_for_paths_timeout_seconds: int = 1

    dual_vm_instances: tuple[str, ...] = ("a", "b")
    opencode_guest_proxy_port: int = 14080
    opencode_vsock_port: int = 43181
    proxy_max_header_bytes: int = 16 * 1024
    proxy_receive_bytes: int = 65536
    vm_hold_seconds: int = 600
    guest_ready_attempts: int = 180
    guest_ready_poll_seconds: float = 0.5
    workload_bundle_path: str = "local/kata/workload-bundle"
    capability_bundle_path: str = "local/kata/guest-bundle"
    provider_port: int = 18098
    provider_host: str = "127.0.0.1"
    local_api_key: str = "actrail-kata-local-key"
    guest_poll_seconds: int = 1
    provider_delay_seconds: float = 1.0
    llm_provider: str = "deepseek"
    llm_model: str = "deepseek-chat"
    llm_max_tokens: int = 128
    llm_context_window: int = 32768
    llm_reasoning_effort: str = "off"

    opencode_timeout_seconds: int = 180


CASE_CONSTANTS = VirtualContainerXiaooConcurrencyConstants()
