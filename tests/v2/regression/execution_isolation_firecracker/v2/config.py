from __future__ import annotations

from typing import ClassVar

from tests.v2.regression.execution_isolation_cloud_hypervisor.v2.config import (
    CloudHypervisorExecutionIsolationConfig,
)

from .identity import FirecrackerScenarioIdentity


class FirecrackerExecutionIsolationConfig(
    CloudHypervisorExecutionIsolationConfig
):
    CONFIG_NAMESPACE: ClassVar[str] = "EXECUTION_ISOLATION_FIRECRACKER"
    ENVIRONMENT_PREFIX: ClassVar[str] = "EXECUTION_ISOLATION_FIRECRACKER_E2E_"
    BACKEND: ClassVar[str] = "firecracker"
    VMM_COMMAND: ClassVar[str] = "firecracker"
    PROFILE_RUNTIME_CONFIG: ClassVar[str] = (
        "VIRTUAL_CONTAINER_E2E_FIRECRACKER_DATA_CONFIG"
    )
    DEFAULT_VM_ROOT: ClassVar[str] = "/run/vc/firecracker"
    SUPPORTED_ARCHITECTURES: ClassVar[frozenset[str]] = frozenset(
        {"aarch64", "x86_64"}
    )
    IDENTITY: ClassVar[type[FirecrackerScenarioIdentity]] = (
        FirecrackerScenarioIdentity
    )
