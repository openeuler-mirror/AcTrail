from __future__ import annotations

from dataclasses import dataclass

from tests.e2e.common.process import ManagedProcess
from tests.e2e.common.sandbox_alert_database import SandboxAlertRecord


@dataclass
class OwnedProcesses:
    gateway: ManagedProcess | None = None
    sandbox_agent: ManagedProcess | None = None
    workload: ManagedProcess | None = None


@dataclass(frozen=True)
class ScenarioOutcome:
    records: dict[str, SandboxAlertRecord]
    initial_connection: str
    reconnection: str
