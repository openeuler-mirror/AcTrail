from __future__ import annotations

from typing import ClassVar

from tests.v2.common.execution_isolation.kata.config import KataExecutionIsolationConfig

from .constants import CASE_CONSTANTS
from .identity import CloudHypervisorScenarioIdentity


class CloudHypervisorExecutionIsolationConfig(KataExecutionIsolationConfig):
    CONFIG_NAMESPACE: ClassVar[str] = "EXECUTION_ISOLATION_CLOUD_HYPERVISOR"
    ENVIRONMENT_PREFIX: ClassVar[str] = (
        "EXECUTION_ISOLATION_CLOUD_HYPERVISOR_E2E_"
    )
    BACKEND: ClassVar[str] = "cloud-hypervisor"
    VMM_COMMAND: ClassVar[str] = "cloud-hypervisor"
    PROFILE_RUNTIME_CONFIG: ClassVar[str] = (
        "VIRTUAL_CONTAINER_E2E_CLOUD_HYPERVISOR_DATA_CONFIG"
    )
    DEFAULT_VM_ROOT: ClassVar[str] = CASE_CONSTANTS.vm_root
    SUPPORTED_ARCHITECTURES: ClassVar[frozenset[str]] = frozenset(
        {"aarch64", "x86_64"}
    )
    IDENTITY: ClassVar[type[CloudHypervisorScenarioIdentity]] = (
        CloudHypervisorScenarioIdentity
    )
