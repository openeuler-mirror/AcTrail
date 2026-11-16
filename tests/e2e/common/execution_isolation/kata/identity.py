from __future__ import annotations


class ExecutionIsolationIdentity:
    CASE: str
    DISPLAY: str
    PLUGIN_INSTANCE: str
    SUBSCRIPTION: str
    XIAOO_RESPONSE_MARKER: str
    NAMED_ROOT_MARKER: str
    AGENT_TOOLS_MARKER: str
    OOM_KILL_MARKER: str
    AGENT_READ_INPUT_MARKER: str
    AGENT_WRITE_MARKER: str

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
