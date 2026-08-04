from __future__ import annotations

import os
import shutil
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.config import TestCaseInputs


@dataclass(frozen=True)
class ContainerAgentXiaooConfig(TestCaseInputs):
    xiaoo_bin: Path | None
    image: str
    timeout_seconds: int
    cleanup_grace_seconds: int

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "ContainerAgentXiaooConfig":
        raw_xiaoo = os.environ.get(
            "CONTAINER_AGENT_XIAOO_BINARY",
            os.environ.get("XIAOO_BINARY", ""),
        )
        if not raw_xiaoo:
            raw_xiaoo = shutil.which("xiaoo") or ""
        xiaoo_bin = Path(raw_xiaoo).expanduser() if raw_xiaoo else None

        timeout_seconds = int(
            os.environ.get("CONTAINER_AGENT_XIAOO_E2E_TIMEOUT_SECONDS", "900")
        )
        cleanup_grace_seconds = int(
            os.environ.get(
                "CONTAINER_AGENT_XIAOO_E2E_CLEANUP_GRACE_SECONDS",
                "30",
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
            repo=inputs.repo,
            bin_dir=inputs.bin_dir,
            work_dir=inputs.work_dir,
            xiaoo_bin=xiaoo_bin,
            image=os.environ.get(
                "CONTAINER_AGENT_XIAOO_IMAGE",
                "ubuntu:24.04",
            ),
            timeout_seconds=timeout_seconds,
            cleanup_grace_seconds=cleanup_grace_seconds,
        )
