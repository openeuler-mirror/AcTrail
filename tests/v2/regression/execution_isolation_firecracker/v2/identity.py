from __future__ import annotations

from tests.v2.common.execution_isolation.kata.identity import (
    ExecutionIsolationIdentity,
)


class FirecrackerScenarioIdentity(ExecutionIsolationIdentity):
    CASE = "execution-isolation-firecracker"
    DISPLAY = "Firecracker execution-isolation"
    PLUGIN_INSTANCE = f"{CASE}.resource-alert"
    SUBSCRIPTION = f"{CASE}-alerts"
    XIAOO_RESPONSE_MARKER = "ACTRAIL_FIRECRACKER_XIAOO_OK"
    NAMED_ROOT_MARKER = "ACTRAIL_FIRECRACKER_NAMED_ROOT_OK"
    AGENT_TOOLS_MARKER = "ACTRAIL_FIRECRACKER_AGENT_TOOLS_OK"
    OOM_KILL_MARKER = "ACTRAIL_FIRECRACKER_OOM_KILL_OK"
    AGENT_READ_INPUT_MARKER = "ACTRAIL_FIRECRACKER_AGENT_READ_INPUT"
    AGENT_WRITE_MARKER = "ACTRAIL_FIRECRACKER_AGENT_WRITE_OK"
