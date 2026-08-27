from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

from .artifacts import CompositeArtifactRequirement, DirectoryManifestRequirement
from .backend import kata_backend
from .container import KataTestContainer
from .image import ContainerdImage, PullPolicy
from .process import CommandRunner
from .requirements import (
    KataContainerRequirements,
    KataMount,
    KataRuntimeProfile,
    PreparePolicy,
    RequirementCheck,
)


@dataclass(frozen=True)
class KataRequirementsBuilder:
    backend: str
    runtime: str
    runtime_config: Path
    image: str
    runner: CommandRunner
    pull_policy: PullPolicy
    image_archive: Path | None
    runtime_timeout_seconds: float
    uid: int
    gid: int
    ready_timeout_seconds: float
    snapshotter: str | None = None
    image_archive_sha256: str | None = None

    def build(
        self,
        *,
        name_prefix: str,
        command: tuple[str, ...],
        mounts: tuple[KataMount, ...],
        artifact_directories: tuple[Path, ...],
        labels: tuple[tuple[str, str], ...],
        running_validator: Callable[[KataTestContainer], RequirementCheck],
        privileged_without_host_devices: bool = False,
    ) -> KataContainerRequirements:
        backend = kata_backend(self.backend)
        snapshotter = self.snapshotter
        if snapshotter is None:
            snapshotter = backend.default_snapshotter
        prepare_policy = (
            PreparePolicy.CHECK_ONLY
            if self.pull_policy is PullPolicy.NEVER
            else PreparePolicy.REFRESH_INVALID
        )
        return KataContainerRequirements(
            profile=KataRuntimeProfile(
                backend=self.backend,
                namespace="default",
                runtime=self.runtime,
                runtime_config=self.runtime_config,
                image=self.image,
                snapshotter=snapshotter,
            ),
            image=ContainerdImage(
                reference=self.image,
                runner=self.runner,
                pull_policy=self.pull_policy,
                archive=self.image_archive,
                archive_sha256=self.image_archive_sha256,
                timeout_seconds=self.runtime_timeout_seconds,
                snapshotter=snapshotter,
            ),
            artifact_requirement=CompositeArtifactRequirement(
                tuple(
                    DirectoryManifestRequirement(directory)
                    for directory in artifact_directories
                )
            ),
            name_prefix=name_prefix,
            command=command,
            mounts=mounts,
            uid=self.uid,
            gid=self.gid,
            labels=labels,
            privileged_without_host_devices=(
                privileged_without_host_devices
            ),
            ready_timeout_seconds=self.ready_timeout_seconds,
            prepare_policy=prepare_policy,
            running_validator=running_validator,
        )
