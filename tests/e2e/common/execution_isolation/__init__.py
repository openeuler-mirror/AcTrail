from .alert_path import SandboxAlertPath, SandboxAlertThresholds
from .evidence_database import SandboxEvidenceDatabase
from .sandbox_agent import SandboxAgentProfile, SandboxAgentTiming
from .resource_alert_web import SandboxResourceAlertWebControl

_CONTROLLED_OOM_EXPORTS = {
    "ControlledHostOom",
    "ControlledHostOomResult",
    "MonitoredRootMarker",
    "memory_cgroup_problem",
}


def __getattr__(name: str):
    if name not in _CONTROLLED_OOM_EXPORTS:
        raise AttributeError(name)
    from . import controlled_oom

    value = getattr(controlled_oom, name)
    globals()[name] = value
    return value

__all__ = [
    "SandboxAgentProfile",
    "SandboxAgentTiming",
    "SandboxResourceAlertWebControl",
    "SandboxAlertPath",
    "SandboxAlertThresholds",
    "SandboxEvidenceDatabase",
    "ControlledHostOom",
    "ControlledHostOomResult",
    "MonitoredRootMarker",
    "memory_cgroup_problem",
]
