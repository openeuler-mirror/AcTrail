"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass


@dataclass(frozen=True)
class SandboxResourceAlertHostAssetsConstants:
    actrail_host_child_timeout_seconds: int = 90
    wait_for_release_poll_seconds: float = 0.05


CASE_CONSTANTS = SandboxResourceAlertHostAssetsConstants()
