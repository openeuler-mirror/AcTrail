from __future__ import annotations

from typing import Any


class EbpfOnNotifyOffCase:
    suffix = "host-only"
    expected_profile = "container-auto-ebpf-on-notify-off"
    expected_host = "enabled"
    expected_notify = "disabled"
    custom_seccomp = False
    host_ebpf_enabled = True
    progress_step = "ebpf_on_notify_off"
    progress_message = "checking host eBPF enabled and seccomp notify disabled"

    def run(self, scenario: Any) -> None:
        scenario.run_matrix_case(self)
