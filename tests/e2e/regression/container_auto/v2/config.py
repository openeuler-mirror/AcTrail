from __future__ import annotations

import os
from pathlib import Path
from dataclasses import dataclass

from tests.e2e.common.core import TestRunDirectories

from .constants import CASE_CONSTANTS


@dataclass(frozen=True)
class ContainerAutoConfig:
    repo: Path
    bin_dir: Path
    work_dir: Path
    base_image: str
    timeout_seconds: int
    cleanup_grace_seconds: int
    rebuild_image: bool

    @classmethod
    def from_environment(cls, directories: TestRunDirectories) -> "ContainerAutoConfig":
        timeout_seconds = int(
            os.environ.get("CONTAINER_AUTO_E2E_TIMEOUT_SECONDS", str(CASE_CONSTANTS.timeout_seconds))
        )
        cleanup_grace_seconds = int(
            os.environ.get("CONTAINER_AUTO_E2E_CLEANUP_GRACE_SECONDS", str(CASE_CONSTANTS.cleanup_grace_seconds))
        )
        if timeout_seconds <= 0 or cleanup_grace_seconds <= 0:
            raise ValueError("container-auto timeouts must be positive")
        return cls(
            repo=directories.repo,
            bin_dir=directories.bin_dir,
            work_dir=directories.work_dir,
            base_image=os.environ.get(
                "CONTAINER_AUTO_E2E_BASE_IMAGE",
                CASE_CONSTANTS.base_image,
            ),
            timeout_seconds=timeout_seconds,
            cleanup_grace_seconds=cleanup_grace_seconds,
            rebuild_image=os.environ.get(
                "CONTAINER_AUTO_E2E_REBUILD_IMAGE", str(int(CASE_CONSTANTS.rebuild_image))
            )
            == "1",
        )
