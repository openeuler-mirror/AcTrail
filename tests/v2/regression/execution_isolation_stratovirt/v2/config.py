from __future__ import annotations

from typing import ClassVar

from tests.v2.regression.execution_isolation_cloud_hypervisor.v2.config import (
    CloudHypervisorExecutionIsolationConfig,
)

from .identity import StratoVirtScenarioIdentity


class StratoVirtExecutionIsolationConfig(
    CloudHypervisorExecutionIsolationConfig
):
    CONFIG_NAMESPACE: ClassVar[str] = "EXECUTION_ISOLATION_STRATOVIRT"
    ENVIRONMENT_PREFIX: ClassVar[str] = "EXECUTION_ISOLATION_STRATOVIRT_E2E_"
    BACKEND: ClassVar[str] = "stratovirt"
    VMM_COMMAND: ClassVar[str] = "stratovirt"
    PROFILE_RUNTIME_CONFIG: ClassVar[str] = (
        "VIRTUAL_CONTAINER_E2E_STRATOVIRT_DATA_CONFIG"
    )
    SUPPORTED_ARCHITECTURES: ClassVar[frozenset[str]] = frozenset(
        {"aarch64", "x86_64"}
    )
    IDENTITY: ClassVar[type[StratoVirtScenarioIdentity]] = (
        StratoVirtScenarioIdentity
    )
