from __future__ import annotations

from typing import Any


class EbpfOffNotifyOffCase:
    suffix = "neither"
    expected_profile = "container-auto-ebpf-off-notify-off"
    expected_host = "disabled"
    expected_notify = "disabled"
    custom_seccomp = False
    host_ebpf_enabled = False
    progress_step = "Check capture with eBPF and seccomp notify disabled"
    progress_message = "checking host eBPF disabled and seccomp notify disabled"

    def run(self, scenario: Any) -> None:
        scenario.run_matrix_case(self)
