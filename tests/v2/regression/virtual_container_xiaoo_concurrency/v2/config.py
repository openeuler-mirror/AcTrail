from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import CommonTestConfig, TestCaseInputs
from tests.v2.common.kata_runtime.environment import (
    absolute_path,
    bounded_environment_int,
    optional_absolute_path,
    positive_environment_int,
)
from tests.v2.common.kata_runtime.image import PullPolicy
from tests.v2.regression.virtual_container.v2.config import SUPPORTED_BACKENDS


@dataclass(frozen=True)
class VirtualContainerXiaooConcurrencyConfig(CommonTestConfig):
    backend: str
    runtime_config: Path | None
    ctr_runtime: str
    image: str
    image_pull_policy: PullPolicy
    image_archive: Path | None
    workload_bundle: Path
    capability_bundle: Path
    xiaoo_binary: Path | None
    runtime_timeout_seconds: int
    ready_timeout_seconds: int
    overlap_timeout_seconds: int
    settle_seconds: int
    workload_uid: int
    workload_gid: int
    artifact_manifest: Path | None

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> VirtualContainerXiaooConcurrencyConfig:
        common = CommonTestConfig.from_environment(
            inputs,
            "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY",
        )
        prefix = "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_"
        backend = os.environ.get(
            f"{prefix}BACKEND",
            os.environ.get("BACKEND", "stratovirt"),
        )
        if backend not in SUPPORTED_BACKENDS:
            raise ValueError(f"unsupported Kata backend: {backend}")
        backend_env = backend.upper().replace("-", "_")
        runtime_config = optional_absolute_path(
            os.environ.get(f"{prefix}RUNTIME_CONFIG")
            or os.environ.get("RUNTIME_CONFIG_PATH")
            or os.environ.get(
                f"VIRTUAL_CONTAINER_E2E_{backend_env}_DATA_CONFIG"
            ),
            f"{prefix}RUNTIME_CONFIG",
        )
        xiaoo_binary = optional_absolute_path(
            os.environ.get(f"{prefix}XIAOO_BINARY")
            or os.environ.get("XIAOO_E2E_BINARY"),
            f"{prefix}XIAOO_BINARY",
        )

        pull_value = os.environ.get(
            f"{prefix}IMAGE_PULL_POLICY",
            os.environ.get("VIRTUAL_CONTAINER_E2E_IMAGE_PULL_POLICY", "never"),
        )
        try:
            pull_policy = PullPolicy(pull_value)
        except ValueError as error:
            raise ValueError(
                f"{prefix}IMAGE_PULL_POLICY must be never, missing or always"
            ) from error

        repo = inputs.repo.resolve()
        return cls(
            **common.as_kwargs(),
            backend=backend,
            runtime_config=runtime_config,
            ctr_runtime=os.environ.get(
                f"{prefix}CTR_RUNTIME",
                os.environ.get("CTR_RUNTIME", "io.containerd.kata.v2"),
            ),
            image=os.environ.get(
                f"{prefix}IMAGE",
                os.environ.get(
                    "VIRTUAL_CONTAINER_E2E_IMAGE",
                    "docker.io/library/actrail-openeuler-workload:24.09",
                ),
            ),
            image_pull_policy=pull_policy,
            image_archive=optional_absolute_path(
                os.environ.get(f"{prefix}IMAGE_ARCHIVE")
                or os.environ.get("VIRTUAL_CONTAINER_E2E_IMAGE_ARCHIVE"),
                f"{prefix}IMAGE_ARCHIVE",
            ),
            workload_bundle=absolute_path(
                os.environ.get(
                    f"{prefix}WORKLOAD_BUNDLE",
                    os.environ.get(
                        "WORKLOAD_BUNDLE_DIR",
                        str(repo / "local/kata/workload-bundle"),
                    ),
                ),
                f"{prefix}WORKLOAD_BUNDLE",
            ),
            capability_bundle=absolute_path(
                os.environ.get(
                    "CAPABILITY_BUNDLE_DIR",
                    str(repo / "local/kata/guest-bundle"),
                ),
                "CAPABILITY_BUNDLE_DIR",
            ),
            xiaoo_binary=xiaoo_binary,
            runtime_timeout_seconds=positive_environment_int(
                f"{prefix}RUNTIME_TIMEOUT_SECONDS",
                "900",
            ),
            ready_timeout_seconds=positive_environment_int(
                f"{prefix}READY_TIMEOUT_SECONDS",
                "90",
            ),
            overlap_timeout_seconds=positive_environment_int(
                f"{prefix}OVERLAP_TIMEOUT_SECONDS",
                "30",
            ),
            settle_seconds=bounded_environment_int(
                f"{prefix}SETTLE_SECONDS",
                "5",
                minimum=0,
                maximum=300,
            ),
            workload_uid=bounded_environment_int(
                f"{prefix}WORKLOAD_UID",
                "1000",
                minimum=0,
                maximum=2147483647,
            ),
            workload_gid=bounded_environment_int(
                f"{prefix}WORKLOAD_GID",
                "39000",
                minimum=0,
                maximum=2147483647,
            ),
            artifact_manifest=optional_absolute_path(
                os.environ.get(f"{prefix}ARTIFACT_MANIFEST")
                or os.environ.get("VIRTUAL_CONTAINER_E2E_ARTIFACT_MANIFEST"),
                f"{prefix}ARTIFACT_MANIFEST",
            ),
        )
