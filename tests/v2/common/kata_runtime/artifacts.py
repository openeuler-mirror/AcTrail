"""V2 runner resolution of deployment artifacts and mutable test bundles.

The published artifact format is owned by the deployment tools in
deploy/virtual-container/host/v2_artifacts_support/manifest.py.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path, PurePosixPath

from v2_artifacts_support.manifest import (
    GUEST_RELEASE_FILES,
    DeploymentArtifacts,
    verify_directory_manifest,
    verify_file_digest,
)
from v2_artifacts_support.model import RELEASE_FILES

from .checksums import sha256_file
from .process import CommandRunner
from .requirements import ArtifactRequirement, PreparePolicy

from .constants import DEFAULTS


def resolve_deployment_artifacts(
    manifest: Path | None,
    *,
    bin_dir: Path,
    guest_bundle: Path,
    workload_bundle: Path,
    expected_backend: str,
    expected_runtime: str,
    expected_workload_image: str,
) -> DeploymentArtifacts | None:
    """Resolve either a format-2 manifest or the deprecated mutable bundles."""

    if manifest is None:
        validate_release_bundle_consistency(
            bin_dir,
            guest_bundle,
            workload_bundle,
        )
        return None
    try:
        deployment = DeploymentArtifacts.load(
            manifest,
            bin_dir=bin_dir,
            expected_backend=expected_backend,
            expected_runtime=expected_runtime,
        )
    except (OSError, RuntimeError, ValueError) as error:
        raise RuntimeError(str(error)) from error
    if deployment.workload_image != expected_workload_image:
        raise RuntimeError(
            "workload image mismatch: "
            f"profile={expected_workload_image} "
            f"manifest={deployment.workload_image}"
        )
    return deployment


def validate_release_bundle_consistency(
    bin_dir: Path,
    guest_bundle: Path,
    workload_bundle: Path,
) -> None:
    """Validate deprecated mutable bundle paths against the current release."""

    DirectoryManifestRequirement(guest_bundle).ensure(PreparePolicy.CHECK_ONLY)
    DirectoryManifestRequirement(workload_bundle).ensure(PreparePolicy.CHECK_ONLY)
    for filename in RELEASE_FILES.values():
        release = bin_dir / filename
        if not release.is_file():
            raise RuntimeError(f"current release artifact is missing: {release}")
    for filename in GUEST_RELEASE_FILES.values():
        release = bin_dir / filename
        verify_file_digest(
            guest_bundle / filename,
            sha256_file(release),
            f"guest bundle {filename}",
        )
    verify_file_digest(
        workload_bundle / "bin/actrailctl",
        sha256_file(bin_dir / "actrailctl"),
        "workload bundle actrailctl",
    )


@dataclass
class DirectoryManifestRequirement:
    directory: Path
    manifest_name: str = DEFAULTS.manifest_name
    prepare_command: tuple[str, ...] | None = None
    runner: CommandRunner | None = None
    timeout_seconds: float = DEFAULTS.artifact_timeout_seconds
    _digest: str | None = field(default=None, init=False, repr=False)

    def __post_init__(self) -> None:
        if not self.directory.is_absolute():
            raise ValueError(
                f"artifact directory must be absolute: {self.directory}"
            )
        manifest_path = PurePosixPath(self.manifest_name)
        if manifest_path.is_absolute() or ".." in manifest_path.parts:
            raise ValueError(f"unsafe artifact manifest name: {self.manifest_name}")
        if self.prepare_command is not None and (
            not self.prepare_command or any(not item for item in self.prepare_command)
        ):
            raise ValueError("artifact prepare command must contain non-empty argv")
        if (self.prepare_command is None) != (self.runner is None):
            raise ValueError(
                "artifact prepare command and command runner must be "
                "configured together"
            )
        if self.timeout_seconds <= 0:
            raise ValueError("artifact preparation timeout must be positive")

    @property
    def digest(self) -> str:
        if self._digest is None:
            raise RuntimeError("artifact requirement has not been ensured")
        return self._digest

    def ensure(self, policy: PreparePolicy) -> None:
        manifest = self.directory / self.manifest_name
        if not manifest.is_file():
            if policy is PreparePolicy.CHECK_ONLY or self.prepare_command is None:
                raise FileNotFoundError(f"artifact manifest is missing: {manifest}")
            assert self.runner is not None
            result = self.runner.run(
                self.prepare_command,
                timeout=self.timeout_seconds,
            )
            if result.returncode != 0:
                raise RuntimeError(
                    f"failed to prepare artifact exit={result.returncode}: "
                    f"{result.diagnostic or 'no diagnostic output'}"
                )
            if not manifest.is_file():
                raise RuntimeError(
                    "artifact preparation completed without producing manifest: "
                    f"{manifest}"
                )
        self._digest = verify_directory_manifest(self.directory, manifest)


@dataclass(frozen=True)
class CompositeArtifactRequirement:
    requirements: tuple[ArtifactRequirement, ...]

    def ensure(self, policy: PreparePolicy) -> None:
        for requirement in self.requirements:
            requirement.ensure(policy)
