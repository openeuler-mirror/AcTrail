"""Runtime defaults for this execution-isolation backend."""

from dataclasses import dataclass


@dataclass(frozen=True)
class StratovirtConstants:
    vm_root: str = '/run/vc/vm'


CASE_CONSTANTS = StratovirtConstants()
