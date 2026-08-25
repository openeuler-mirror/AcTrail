from __future__ import annotations


class CloudHypervisorScenarioIdentity:
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

    @classmethod
    def subscriber_client(cls, run_token: str) -> str:
        return f"{cls.CASE}-{run_token}"

    @classmethod
    def workload_markers(cls) -> tuple[str, ...]:
        return (
            f"KATA_XIAOO_PROVIDER_READY instance={cls.CASE}",
            cls.XIAOO_RESPONSE_MARKER,
            cls.NAMED_ROOT_MARKER,
            f"{cls.AGENT_TOOLS_MARKER} instance={cls.CASE}",
            f"KATA_XIAOO_WORKLOAD_OK instance={cls.CASE}",
        )

    @classmethod
    def failure(cls, message: str) -> str:
        return f"{cls.DISPLAY}: {message}"
