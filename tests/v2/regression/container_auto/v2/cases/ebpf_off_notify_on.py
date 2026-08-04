from __future__ import annotations

from typing import Any


class EbpfOffNotifyOnCase:
    suffix = "notify-only"
    expected_profile = "container-auto-ebpf-off-notify-on"
    expected_host = "disabled"
    expected_notify = "enabled"
    custom_seccomp = True
    host_ebpf_enabled = False
    progress_step = "ebpf_off_notify_on"
    progress_message = "checking host eBPF disabled and seccomp notify enabled"

    def run(self, scenario: Any) -> None:
        scenario.run_matrix_case(self)
