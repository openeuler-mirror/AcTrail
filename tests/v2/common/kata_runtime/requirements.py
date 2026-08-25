from __future__ import annotations

import re
from collections.abc import Callable
from dataclasses import dataclass
from enum import Enum
from pathlib import Path, PurePosixPath
from typing import TYPE_CHECKING, Protocol

if TYPE_CHECKING:
    from .container import KataTestContainer


class PreparePolicy(str, Enum):
    CHECK_ONLY = "check-only"
    MISSING = "missing"
    REFRESH_INVALID = "refresh-invalid"


@dataclass(frozen=True)
class ResolvedImage:
    reference: str
    digest: str | None = None

    def __post_init__(self) -> None:
        if not self.reference:
            raise ValueError("resolved image reference must not be empty")


@dataclass(frozen=True)
class RequirementCheck:
    ready: bool
    refreshable: bool
    reason: str

    @classmethod
    def ready_check(cls) -> RequirementCheck:
        return cls(ready=True, refreshable=False, reason="ready")

    @classmethod
    def not_ready(
        cls,
        reason: str,
        *,
        refreshable: bool,
    ) -> RequirementCheck:
        if not reason:
            raise ValueError("failed requirement check must include a reason")
        return cls(ready=False, refreshable=refreshable, reason=reason)

    def __post_init__(self) -> None:
        if self.ready and self.refreshable:
            raise ValueError("a ready requirement check cannot be refreshable")


@dataclass(frozen=True)
class KataMount:
    source: Path
    target: str
    read_only: bool = True

    def __post_init__(self) -> None:
        if not self.source.is_absolute():
            raise ValueError(f"Kata mount source must be absolute: {self.source}")
        if not PurePosixPath(self.target).is_absolute():
            raise ValueError(f"Kata mount target must be absolute: {self.target}")


@dataclass(frozen=True)
class KataCreateSpec:
    namespace: str
    runtime: str
    runtime_config: Path | None
    image: ResolvedImage
    command: tuple[str, ...]
    uid: int
    gid: int
    mounts: tuple[KataMount, ...] = ()
    labels: tuple[tuple[str, str], ...] = ()
    environment: tuple[tuple[str, str], ...] = ()
    ready_timeout_seconds: float = 60
    privileged_without_host_devices: bool = False
    snapshotter: str | None = None

    def __post_init__(self) -> None:
        if not self.namespace:
            raise ValueError("containerd namespace must not be empty")
        if not self.runtime:
            raise ValueError("Kata runtime must not be empty")
        if self.runtime_config is not None and not self.runtime_config.is_absolute():
            raise ValueError(
                f"Kata runtime config must be absolute: {self.runtime_config}"
            )
        if not self.command or any(not value for value in self.command):
            raise ValueError("Kata workload command must contain non-empty argv")
        if self.uid < 0 or self.gid < 0:
            raise ValueError("Kata workload UID/GID must be non-negative")
        if self.ready_timeout_seconds <= 0:
            raise ValueError("Kata ready timeout must be positive")
        if self.snapshotter is not None and not self.snapshotter.strip():
            raise ValueError("containerd snapshotter must not be empty")
        if not isinstance(self.privileged_without_host_devices, bool):
            raise ValueError(
                "privileged_without_host_devices must be a boolean"
            )


class ImageRequirement(Protocol):
    def ensure(self, policy: PreparePolicy) -> ResolvedImage: ...

    def refresh(self, reason: str) -> ResolvedImage: ...


class ContainerRequirements(Protocol):
    name_prefix: str
    prepare_policy: PreparePolicy
    image: ImageRequirement

    def validate_static(self) -> None: ...

    def create_spec(self, image: ResolvedImage) -> KataCreateSpec: ...

    def validate_running(
        self,
        container: KataTestContainer,
    ) -> RequirementCheck: ...

    def refresh(self, reason: str) -> ResolvedImage: ...


class ArtifactRequirement(Protocol):
    def ensure(self, policy: PreparePolicy) -> None: ...


@dataclass(frozen=True)
class KataRuntimeProfile:
    backend: str
    namespace: str
    runtime: str
    runtime_config: Path
    image: str
    snapshotter: str | None = None

    def __post_init__(self) -> None:
        if not self.backend:
            raise ValueError("Kata backend must not be empty")
        if not self.namespace:
            raise ValueError("containerd namespace must not be empty")
        if not self.runtime:
            raise ValueError("Kata runtime must not be empty")
        if not self.runtime_config.is_absolute():
            raise ValueError(
                f"Kata runtime config must be absolute: {self.runtime_config}"
            )
        if not self.image:
            raise ValueError("Kata workload image must not be empty")
        if self.snapshotter is not None and not self.snapshotter.strip():
            raise ValueError("containerd snapshotter must not be empty")


_SAFE_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
_SAFE_ENVIRONMENT_NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")


@dataclass(frozen=True)
class KataContainerRequirements:
    """Immutable case requirements consumed by :class:`KataTestContainer`."""

    profile: KataRuntimeProfile
    image: ImageRequirement
    name_prefix: str
    command: tuple[str, ...]
    mounts: tuple[KataMount, ...]
    uid: int
    gid: int
    labels: tuple[tuple[str, str], ...] = ()
    environment: tuple[tuple[str, str], ...] = ()
    privileged_without_host_devices: bool = False
    ready_timeout_seconds: float = 60
    prepare_policy: PreparePolicy = PreparePolicy.CHECK_ONLY
    artifact_requirement: ArtifactRequirement | None = None
    running_validator: (
        Callable[[KataTestContainer], RequirementCheck] | None
    ) = None

    def __post_init__(self) -> None:
        if not _SAFE_NAME.fullmatch(self.name_prefix):
            raise ValueError(
                "Kata name prefix must contain only letters, digits, dot, "
                f"underscore or dash: {self.name_prefix}"
            )
        if not self.command or any(not value for value in self.command):
            raise ValueError("Kata command must contain non-empty argv")
        if self.uid < 0 or self.gid < 0:
            raise ValueError("Kata workload UID/GID must be non-negative")
        if self.ready_timeout_seconds <= 0:
            raise ValueError("Kata ready timeout must be positive")
        if not isinstance(self.prepare_policy, PreparePolicy):
            raise ValueError(
                f"unsupported Kata prepare policy: {self.prepare_policy}"
            )
        for name, value in self.labels:
            if not name or not value:
                raise ValueError("Kata labels must contain non-empty name/value")
        for name, value in self.environment:
            if not _SAFE_ENVIRONMENT_NAME.fullmatch(name):
                raise ValueError(f"invalid Kata environment name: {name}")
            if "\x00" in value:
                raise ValueError(f"Kata environment value contains NUL: {name}")
        if not isinstance(self.privileged_without_host_devices, bool):
            raise ValueError(
                "privileged_without_host_devices must be a boolean"
            )

    def validate_static(self) -> None:
        if not self.profile.runtime_config.is_file():
            raise FileNotFoundError(
                f"Kata runtime config does not exist: {self.profile.runtime_config}"
            )
        if self.artifact_requirement is not None:
            self.artifact_requirement.ensure(self.prepare_policy)

    def create_spec(self, image: ResolvedImage) -> KataCreateSpec:
        if image.reference != self.profile.image:
            raise RuntimeError(
                "resolved workload image does not match the Kata profile: "
                f"resolved={image.reference} profile={self.profile.image}"
            )
        return KataCreateSpec(
            namespace=self.profile.namespace,
            runtime=self.profile.runtime,
            runtime_config=self.profile.runtime_config,
            image=image,
            command=self.command,
            uid=self.uid,
            gid=self.gid,
            mounts=self.mounts,
            labels=self.labels,
            environment=self.environment,
            ready_timeout_seconds=self.ready_timeout_seconds,
            privileged_without_host_devices=(
                self.privileged_without_host_devices
            ),
            snapshotter=self.profile.snapshotter,
        )

    def validate_running(
        self,
        container: KataTestContainer,
    ) -> RequirementCheck:
        if self.running_validator is not None:
            return self.running_validator(container)
        if container.is_running():
            return RequirementCheck.ready_check()
        return RequirementCheck.not_ready(
            "Kata task is not running",
            refreshable=False,
        )

    def refresh(self, reason: str) -> ResolvedImage:
        return self.image.refresh(reason)
