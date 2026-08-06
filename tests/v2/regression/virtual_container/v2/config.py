from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.config import CommonTestConfig, TestCaseInputs
from tests.v2.common.kata_runtime.backend import supported_backends
from tests.v2.common.kata_runtime.environment import (
    absolute_path,
    bounded_environment_int,
    optional_absolute_path,
    positive_environment_int,
)
from tests.v2.common.kata_runtime.image import PullPolicy


SUPPORTED_BACKENDS = supported_backends()


@dataclass(frozen=True)
class VirtualContainerConfig(CommonTestConfig):
    scope: str
    backends: tuple[str, ...]
    runtime_timeout_seconds: int
    ready_timeout_seconds: int
    image: str
    image_pull_policy: PullPolicy
    image_archive: Path | None
    settle_seconds: int
    workload_uid: int
    workload_gid: int
    ctr_runtime: str
    runtime_configs: dict[str, Path | None]
    data_configs: dict[str, Path | None]
    kata_config_dirs: tuple[Path, ...]
    artifact_manifest: Path | None
    workload_bundle: Path
    capability_bundle: Path

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> VirtualContainerConfig:
        common = CommonTestConfig.from_environment(inputs, "VIRTUAL_CONTAINER")
        _validate_common_timeouts(common)

        scope = os.environ.get("VIRTUAL_CONTAINER_E2E_SCOPE", "auto")
        if scope not in {"auto", "all", "contracts"}:
            raise ValueError(
                "VIRTUAL_CONTAINER_E2E_SCOPE must be auto, all or contracts"
            )

        backends = _csv(
            os.environ.get(
                "VIRTUAL_CONTAINER_E2E_BACKENDS",
                "stratovirt,cloud-hypervisor",
            )
        )
        if not backends:
            raise ValueError("VIRTUAL_CONTAINER_E2E_BACKENDS must not be empty")
        if len(set(backends)) != len(backends):
            raise ValueError(
                "VIRTUAL_CONTAINER_E2E_BACKENDS must not contain duplicates"
            )
        unsupported = sorted(set(backends) - set(SUPPORTED_BACKENDS))
        if unsupported:
            raise ValueError(
                "unsupported virtual-container backend(s): "
                + ", ".join(unsupported)
            )

        pull_policy_value = os.environ.get(
            "VIRTUAL_CONTAINER_E2E_IMAGE_PULL_POLICY"
        )
        if pull_policy_value is None:
            legacy_pull = os.environ.get("VIRTUAL_CONTAINER_E2E_PULL_IMAGE", "0")
            if legacy_pull not in {"0", "1"}:
                raise ValueError("VIRTUAL_CONTAINER_E2E_PULL_IMAGE must be 0 or 1")
            pull_policy_value = "always" if legacy_pull == "1" else "never"
        try:
            image_pull_policy = PullPolicy(pull_policy_value)
        except ValueError as error:
            raise ValueError(
                "VIRTUAL_CONTAINER_E2E_IMAGE_PULL_POLICY must be "
                "never, missing or always"
            ) from error

        runtime_timeout_seconds = positive_environment_int(
            "VIRTUAL_CONTAINER_E2E_RUNTIME_TIMEOUT_SECONDS",
            "900",
        )
        ready_timeout_seconds = positive_environment_int(
            "VIRTUAL_CONTAINER_E2E_READY_TIMEOUT_SECONDS",
            "90",
        )
        settle_seconds = bounded_environment_int(
            "VIRTUAL_CONTAINER_E2E_SETTLE_SECONDS",
            "5",
            minimum=0,
            maximum=300,
        )
        workload_uid = bounded_environment_int(
            "VIRTUAL_CONTAINER_E2E_WORKLOAD_UID",
            "1000",
            minimum=0,
            maximum=2147483647,
        )
        workload_gid = bounded_environment_int(
            "VIRTUAL_CONTAINER_E2E_WORKLOAD_GID",
            "39000",
            minimum=0,
            maximum=2147483647,
        )

        runtime_configs = {
            backend: optional_absolute_path(
                os.environ.get(
                    f"VIRTUAL_CONTAINER_E2E_{_backend_env(backend)}_CONFIG"
                ),
                f"VIRTUAL_CONTAINER_E2E_{_backend_env(backend)}_CONFIG",
            )
            for backend in backends
        }
        data_configs = {
            backend: optional_absolute_path(
                os.environ.get(
                    f"VIRTUAL_CONTAINER_E2E_{_backend_env(backend)}_DATA_CONFIG"
                ),
                f"VIRTUAL_CONTAINER_E2E_{_backend_env(backend)}_DATA_CONFIG",
            )
            for backend in backends
        }
        config_dirs = tuple(
            absolute_path(value, "KATA_CONFIG_DIRS")
            for value in os.environ.get(
                "KATA_CONFIG_DIRS",
                "/opt/kata/share/defaults/kata-containers:"
                "/usr/share/defaults/kata-containers:"
                "/etc/kata-containers",
            ).split(os.pathsep)
            if value
        )
        if not config_dirs:
            raise ValueError("KATA_CONFIG_DIRS must contain at least one path")

        repo = inputs.repo.resolve()
        return cls(
            **common.as_kwargs(),
            scope=scope,
            backends=backends,
            runtime_timeout_seconds=runtime_timeout_seconds,
            ready_timeout_seconds=ready_timeout_seconds,
            image=os.environ.get(
                "VIRTUAL_CONTAINER_E2E_IMAGE",
                "docker.io/library/actrail-openeuler-workload:24.09",
            ),
            image_pull_policy=image_pull_policy,
            image_archive=optional_absolute_path(
                os.environ.get("VIRTUAL_CONTAINER_E2E_IMAGE_ARCHIVE"),
                "VIRTUAL_CONTAINER_E2E_IMAGE_ARCHIVE",
            ),
            settle_seconds=settle_seconds,
            workload_uid=workload_uid,
            workload_gid=workload_gid,
            ctr_runtime=os.environ.get(
                "CTR_RUNTIME",
                "io.containerd.kata.v2",
            ),
            runtime_configs=runtime_configs,
            data_configs=data_configs,
            kata_config_dirs=config_dirs,
            artifact_manifest=optional_absolute_path(
                os.environ.get("VIRTUAL_CONTAINER_E2E_ARTIFACT_MANIFEST"),
                "VIRTUAL_CONTAINER_E2E_ARTIFACT_MANIFEST",
            ),
            workload_bundle=_path_with_default(
                "WORKLOAD_BUNDLE_DIR",
                repo / "local/kata/workload-bundle",
            ),
            capability_bundle=_path_with_default(
                "CAPABILITY_BUNDLE_DIR",
                repo / "local/kata/guest-bundle",
            ),
        )

    def runtime_config(self, backend: str, *, data: bool = False) -> Path | None:
        if backend not in self.runtime_configs:
            raise ValueError(f"backend was not selected: {backend}")
        if data:
            return self.data_configs[backend] or self.runtime_configs[backend]
        return self.runtime_configs[backend]


def _validate_common_timeouts(config: CommonTestConfig) -> None:
    if config.command_timeout_seconds <= 0:
        raise ValueError(
            "VIRTUAL_CONTAINER_E2E_COMMAND_TIMEOUT_SECONDS must be positive"
        )
    if config.launch_timeout_seconds <= 0:
        raise ValueError(
            "VIRTUAL_CONTAINER_E2E_LAUNCH_TIMEOUT_SECONDS must be positive"
        )
    if config.drain_attempts <= 0 or config.drain_interval_seconds <= 0:
        raise ValueError("VIRTUAL_CONTAINER_E2E drain settings must be positive")


def _csv(value: str) -> tuple[str, ...]:
    return tuple(item.strip() for item in value.split(",") if item.strip())


def _backend_env(backend: str) -> str:
    return backend.upper().replace("-", "_")


def _path_with_default(name: str, default: Path) -> Path:
    value = os.environ.get(name)
    return absolute_path(value, name) if value else default
