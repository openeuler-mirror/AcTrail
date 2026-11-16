"""Shared lifecycle support for Kata-based V2 regression cases."""

import sys
from pathlib import Path

# The artifact format and the Kata config readers belong to the deployment
# tools; the regression runner imports them from there.
_DEPLOY_HOST_TOOLS = str(
    Path(__file__).resolve().parents[4] / "deploy/virtual-container/host"
)
if _DEPLOY_HOST_TOOLS not in sys.path:
    sys.path.append(_DEPLOY_HOST_TOOLS)

from .artifacts import (
    CompositeArtifactRequirement,
    DeploymentArtifacts,
    DirectoryManifestRequirement,
    resolve_deployment_artifacts,
    validate_release_bundle_consistency,
)
from .backend import (
    KataBackend,
    kata_backend,
    shared_filesystem_backends,
    shim_binary,
    supported_backends,
)
from .capabilities import CtrCapabilities
from .checksums import sha256_file
from .container import KataTestContainer
from .factory import KataRequirementsBuilder
from .guest import GuestConsole
from .image import ContainerdImage, PullPolicy, firecracker_workload_reference
from .requirements import (
    ContainerRequirements,
    KataContainerRequirements,
    KataCreateSpec,
    KataMount,
    KataRuntimeProfile,
    PreparePolicy,
    RequirementCheck,
    ResolvedImage,
)
from .runtime_config import (
    REQUIRED_EBPF_KERNEL_CONFIG,
    REQUIRED_FIRECRACKER_KERNEL_CONFIG,
    REQUIRED_VIRTIO_FS_KERNEL_CONFIG,
    discover_kernel_config,
    load_hypervisor_table,
    missing_kernel_config_entries,
    runtime_path,
)

__all__ = [
    "ContainerRequirements",
    "ContainerdImage",
    "CompositeArtifactRequirement",
    "CtrCapabilities",
    "DeploymentArtifacts",
    "DirectoryManifestRequirement",
    "GuestConsole",
    "KataCreateSpec",
    "KataContainerRequirements",
    "KataBackend",
    "KataMount",
    "KataRuntimeProfile",
    "KataRequirementsBuilder",
    "KataTestContainer",
    "PreparePolicy",
    "PullPolicy",
    "REQUIRED_EBPF_KERNEL_CONFIG",
    "REQUIRED_FIRECRACKER_KERNEL_CONFIG",
    "REQUIRED_VIRTIO_FS_KERNEL_CONFIG",
    "RequirementCheck",
    "ResolvedImage",
    "discover_kernel_config",
    "firecracker_workload_reference",
    "kata_backend",
    "load_hypervisor_table",
    "missing_kernel_config_entries",
    "resolve_deployment_artifacts",
    "shared_filesystem_backends",
    "shim_binary",
    "supported_backends",
    "runtime_path",
    "sha256_file",
    "validate_release_bundle_consistency",
]
