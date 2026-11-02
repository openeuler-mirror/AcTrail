from __future__ import annotations


from tests.v2.common.execution_isolation.kata.identity import ExecutionIsolationIdentity


class CloudHypervisorScenarioIdentity(ExecutionIsolationIdentity):
    CASE = "execution-isolation-cloud-hypervisor"
    DISPLAY = "Cloud Hypervisor execution-isolation"
    PLUGIN_INSTANCE = f"{CASE}.resource-alert"
    SUBSCRIPTION = f"{CASE}-alerts"
    XIAOO_RESPONSE_MARKER = "ACTRAIL_CLOUD_HYPERVISOR_XIAOO_OK"
    NAMED_ROOT_MARKER = "ACTRAIL_CLOUD_HYPERVISOR_NAMED_ROOT_OK"
    AGENT_TOOLS_MARKER = "ACTRAIL_CLOUD_HYPERVISOR_AGENT_TOOLS_OK"
    OOM_KILL_MARKER = "ACTRAIL_CLOUD_HYPERVISOR_OOM_KILL_OK"
    AGENT_READ_INPUT_MARKER = "ACTRAIL_CLOUD_HYPERVISOR_AGENT_READ_INPUT"
    AGENT_WRITE_MARKER = "ACTRAIL_CLOUD_HYPERVISOR_AGENT_WRITE_OK"
