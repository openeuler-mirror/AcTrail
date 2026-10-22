"""Runtime defaults for this regression case. Durations are seconds."""

from dataclasses import dataclass


@dataclass(frozen=True)
class VirtualContainerConstants:
    scope: str = 'auto'
    runtime_timeout_seconds: int = 900
    ready_timeout_seconds: int = 90
    settle_seconds: int = 5
    workload_uid: int = 1000
    workload_gid: int = 39000
    backends: str = 'stratovirt,cloud-hypervisor'
    pull_image: bool = False
    image: str = 'docker.io/library/actrail-openeuler-workload:24.09'
    ctr_runtime: str = 'io.containerd.kata.v2'
    kata_config_dirs: str = '/opt/kata/share/defaults/kata-containers:/usr/share/defaults/kata-containers:/etc/kata-containers'
    server_start_settle_seconds: int = 2
    server_completion_timeout_seconds: int = 30
    wait_for_clean_trace_poll_seconds: float = 0.5
    server_exit_timeout_seconds: int = 1

    guest_ready_attempts: int = 180
    guest_ready_poll_seconds: float = 0.5
    workload_bundle_path: str = "local/kata/workload-bundle"
    capability_bundle_path: str = "local/kata/guest-bundle"
    tls_server_hold_seconds: int = 10
    vm_hold_seconds: int = 600


CASE_CONSTANTS = VirtualContainerConstants()
