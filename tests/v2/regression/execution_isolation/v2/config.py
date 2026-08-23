from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import CommonTestConfig, TestCaseInputs
from tests.v2.common.kata_runtime import shim_binary
from tests.v2.common.kata_runtime.environment import (
    absolute_path,
    bounded_environment_int,
    optional_absolute_path,
    positive_environment_int,
)
from tests.v2.common.kata_runtime.image import PullPolicy


@dataclass(frozen=True)
class ExecutionIsolationConfig(CommonTestConfig):
    runtime_config: Path | None
    ctr_runtime: str
    image: str
    image_pull_policy: PullPolicy
    image_archive: Path | None
    artifact_manifest: Path | None
    xiaoo_binary: Path | None
    vm_root: Path
    vsock_port: int
    runtime_timeout_seconds: int
    ready_timeout_seconds: int
    root_discovery_settle_seconds: int
    workload_uid: int
    workload_gid: int

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> ExecutionIsolationConfig:
        common = CommonTestConfig.from_environment(inputs, "EXECUTION_ISOLATION")
        prefix = "EXECUTION_ISOLATION_E2E_"
        backend = os.environ.get(f"{prefix}BACKEND", "cloud-hypervisor")
        if backend != "cloud-hypervisor":
            raise ValueError(
                f"{prefix}BACKEND must be cloud-hypervisor for this harness"
            )

        ctr_runtime = os.environ.get(
            f"{prefix}CTR_RUNTIME",
            os.environ.get("CTR_RUNTIME", "io.containerd.kata.v2"),
        )
        shim_binary(ctr_runtime)
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

        runtime_config = optional_absolute_path(
            os.environ.get(f"{prefix}RUNTIME_CONFIG")
            or os.environ.get(
                "VIRTUAL_CONTAINER_E2E_CLOUD_HYPERVISOR_DATA_CONFIG"
            ),
            f"{prefix}RUNTIME_CONFIG",
        )
        return cls(
            **common.as_kwargs(),
            runtime_config=runtime_config,
            ctr_runtime=ctr_runtime,
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
            artifact_manifest=optional_absolute_path(
                os.environ.get(f"{prefix}ARTIFACT_MANIFEST"),
                f"{prefix}ARTIFACT_MANIFEST",
            ),
            xiaoo_binary=optional_absolute_path(
                os.environ.get(f"{prefix}XIAOO_BINARY")
                or os.environ.get("XIAOO_E2E_BINARY"),
                f"{prefix}XIAOO_BINARY",
            ),
            vm_root=absolute_path(
                os.environ.get(f"{prefix}VM_ROOT", "/run/vc/vm"),
                f"{prefix}VM_ROOT",
            ),
            vsock_port=bounded_environment_int(
                f"{prefix}VSOCK_PORT",
                "43182",
                minimum=1027,
                maximum=65535,
            ),
            runtime_timeout_seconds=positive_environment_int(
                f"{prefix}RUNTIME_TIMEOUT_SECONDS",
                "900",
            ),
            ready_timeout_seconds=positive_environment_int(
                f"{prefix}READY_TIMEOUT_SECONDS",
                "90",
            ),
            root_discovery_settle_seconds=positive_environment_int(
                f"{prefix}ROOT_DISCOVERY_SETTLE_SECONDS",
                "3",
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
        )
