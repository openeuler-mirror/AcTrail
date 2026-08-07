"""Shared lifecycle support for Kata-based V2 regression cases."""

from .artifacts import (
    CompositeArtifactRequirement,
    DeploymentArtifacts,
    DirectoryManifestRequirement,
    resolve_deployment_artifacts,
    validate_release_bundle_consistency,
)
from .backend import KataBackend, kata_backend, shim_binary, supported_backends
from .capabilities import CtrCapabilities
from .checksums import sha256_file
from .container import KataTestContainer
from .factory import KataRequirementsBuilder
from .guest import GuestConsole
from .image import ContainerdImage, PullPolicy
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
from .runtime_config import load_hypervisor_table, runtime_path

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
    "RequirementCheck",
    "ResolvedImage",
    "kata_backend",
    "load_hypervisor_table",
    "resolve_deployment_artifacts",
    "shim_binary",
    "supported_backends",
    "runtime_path",
    "sha256_file",
    "validate_release_bundle_consistency",
]
