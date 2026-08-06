from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

from .artifacts import CompositeArtifactRequirement, DirectoryManifestRequirement
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

    def build(
        self,
        *,
        name_prefix: str,
        command: tuple[str, ...],
        mounts: tuple[KataMount, ...],
        artifact_directories: tuple[Path, ...],
        labels: tuple[tuple[str, str], ...],
        running_validator: Callable[[KataTestContainer], RequirementCheck],
    ) -> KataContainerRequirements:
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
            ),
            image=ContainerdImage(
                reference=self.image,
                runner=self.runner,
                pull_policy=self.pull_policy,
                archive=self.image_archive,
                timeout_seconds=self.runtime_timeout_seconds,
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
            ready_timeout_seconds=self.ready_timeout_seconds,
            prepare_policy=prepare_policy,
            running_validator=running_validator,
        )
