"""Runtime defaults for this execution-isolation backend."""

from dataclasses import dataclass


@dataclass(frozen=True)
class FirecrackerConstants:
    vm_root: str = '/run/vc/firecracker'


CASE_CONSTANTS = FirecrackerConstants()
