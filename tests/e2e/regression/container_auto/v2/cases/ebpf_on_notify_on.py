from __future__ import annotations

from typing import Any


class EbpfOnNotifyOnCase:
    suffix = "both"
    expected_profile = "container-auto-ebpf-on-notify-on"
    expected_host = "enabled"
    expected_notify = "enabled"
    custom_seccomp = True
    host_ebpf_enabled = True
    progress_step = "Check capture with eBPF and seccomp notify enabled"
    progress_message = "checking host eBPF enabled and seccomp notify enabled"

    def run(self, scenario: Any) -> None:
        scenario.run_matrix_case(self)
