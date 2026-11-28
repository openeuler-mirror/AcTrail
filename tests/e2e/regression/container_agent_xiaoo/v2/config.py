from __future__ import annotations

import os
import shutil
from dataclasses import dataclass
from pathlib import Path

from tests.e2e.common.core import TestRunDirectories

from .constants import CASE_CONSTANTS


@dataclass(frozen=True)
class ContainerAgentXiaooConfig:
    repo: Path
    bin_dir: Path
    work_dir: Path
    xiaoo_bin: Path | None
    image: str
    timeout_seconds: int
    cleanup_grace_seconds: int
    rebuild_image: bool

    @classmethod
    def from_environment(
        cls,
        directories: TestRunDirectories,
    ) -> "ContainerAgentXiaooConfig":
        raw_xiaoo = os.environ.get(
            "CONTAINER_AGENT_XIAOO_BINARY",
            os.environ.get("XIAOO_BINARY", ""),
        )
        if not raw_xiaoo:
            raw_xiaoo = shutil.which("xiaoo") or ""
        xiaoo_bin = Path(raw_xiaoo).expanduser() if raw_xiaoo else None

        timeout_seconds = int(
            os.environ.get(
                "CONTAINER_AGENT_XIAOO_E2E_TIMEOUT_SECONDS",
                str(CASE_CONSTANTS.case_timeout_seconds),
            )
        )
        cleanup_grace_seconds = int(
            os.environ.get(
                "CONTAINER_AGENT_XIAOO_E2E_CLEANUP_GRACE_SECONDS",
                str(CASE_CONSTANTS.case_cleanup_grace_seconds),
            )
        )
        if timeout_seconds <= 0:
            raise ValueError(
                "CONTAINER_AGENT_XIAOO_E2E_TIMEOUT_SECONDS must be positive"
            )
        if cleanup_grace_seconds <= 0:
            raise ValueError(
                "CONTAINER_AGENT_XIAOO_E2E_CLEANUP_GRACE_SECONDS must be positive"
            )
        return cls(
            repo=directories.repo,
            bin_dir=directories.bin_dir,
            work_dir=directories.work_dir,
            xiaoo_bin=xiaoo_bin,
            image=os.environ.get(
                "CONTAINER_AGENT_XIAOO_IMAGE",
                CASE_CONSTANTS.image,
            ),
            timeout_seconds=timeout_seconds,
            cleanup_grace_seconds=cleanup_grace_seconds,
            rebuild_image=os.environ.get(
                "CONTAINER_AGENT_XIAOO_E2E_REBUILD_IMAGE",
                "1" if CASE_CONSTANTS.rebuild_image else "0",
            )
            == "1",
        )
