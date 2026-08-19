from __future__ import annotations

import os
from dataclasses import dataclass

from tests.v2.common.core import TestCaseInputs


@dataclass(frozen=True)
class ContainerAutoConfig(TestCaseInputs):
    base_image: str
    timeout_seconds: int
    cleanup_grace_seconds: int
    rebuild_image: bool

    @classmethod
    def from_environment(cls, inputs: TestCaseInputs) -> "ContainerAutoConfig":
        timeout_seconds = int(
            os.environ.get("CONTAINER_AUTO_E2E_TIMEOUT_SECONDS", "1200")
        )
        cleanup_grace_seconds = int(
            os.environ.get("CONTAINER_AUTO_E2E_CLEANUP_GRACE_SECONDS", "30")
        )
        if timeout_seconds <= 0 or cleanup_grace_seconds <= 0:
            raise ValueError("container-auto timeouts must be positive")
        return cls(
            repo=inputs.repo,
            bin_dir=inputs.bin_dir,
            work_dir=inputs.work_dir,
            base_image=os.environ.get(
                "CONTAINER_AUTO_E2E_BASE_IMAGE",
                "openeuler/openeuler:24.03-lts-sp3",
            ),
            timeout_seconds=timeout_seconds,
            cleanup_grace_seconds=cleanup_grace_seconds,
            rebuild_image=os.environ.get(
                "CONTAINER_AUTO_E2E_REBUILD_IMAGE", "0"
            )
            == "1",
        )
