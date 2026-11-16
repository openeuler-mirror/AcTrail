"""Cloud Hypervisor execution-isolation defaults."""

from dataclasses import dataclass


@dataclass(frozen=True)
class CloudHypervisorConstants:
    vm_root: str = "/run/vc/vm"


CASE_CONSTANTS = CloudHypervisorConstants()
