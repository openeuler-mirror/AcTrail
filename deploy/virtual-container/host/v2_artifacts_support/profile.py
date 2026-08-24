"""Machine-local profile writer for V2 virtual-container tests."""

from __future__ import annotations

import json
import os
import pwd
from pathlib import Path
from typing import Protocol

from .io import atomic_json


class ProfileInputs(Protocol):
    backend: str
    bin_dir: Path
    hypervisor: Path
    image_pull_policy: str
    runtime: str
    virtiofsd: Path
    workload_image: str
    workload_image_archive: Path | None


class V2TestProfile:
    _BACKEND_NAMES = {
        "stratovirt": "StratoVirt",
        "cloud-hypervisor": "Cloud Hypervisor",
    }

    @classmethod
    def write(
        cls,
        inputs: ProfileInputs,
        manifest: Path,
        profile_path: Path,
    ) -> None:
        profile = {
            "format": 2,
            "name": (
                "openEuler / Kata 3.32 / "
                + cls._BACKEND_NAMES[inputs.backend]
            ),
            "path_prepend": cls._path_prepend(inputs),
            "environment": cls._environment(inputs, manifest),
        }
        profile_path.parent.mkdir(parents=True, exist_ok=True)
        atomic_json(profile_path, profile)
        profile_path.chmod(0o644)

    @staticmethod
    def validate(profile_path: Path, manifest: Path) -> None:
        try:
            document = json.loads(profile_path.read_text(encoding="utf-8"))
            configured = document["environment"][
                "VIRTUAL_CONTAINER_E2E_ARTIFACT_MANIFEST"
            ]
        except (OSError, KeyError, TypeError, json.JSONDecodeError) as error:
            raise RuntimeError(
                f"cannot validate V2 test profile {profile_path}"
            ) from error
        if configured != str(manifest):
            raise RuntimeError(
                f"V2 test profile points at {configured}, expected {manifest}"
            )

    @staticmethod
    def _environment(
        inputs: ProfileInputs,
        manifest: Path,
    ) -> dict[str, str]:
        environment = {
            "ACTRAIL_BIN_DIR": str(inputs.bin_dir),
            "CTR_RUNTIME": inputs.runtime,
            "VIRTUAL_CONTAINER_E2E_BACKENDS": inputs.backend,
            "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_BACKEND": inputs.backend,
            "VIRTUAL_CONTAINER_E2E_ARTIFACT_MANIFEST": str(manifest),
            "VIRTUAL_CONTAINER_E2E_IMAGE": inputs.workload_image,
            "VIRTUAL_CONTAINER_E2E_IMAGE_PULL_POLICY": inputs.image_pull_policy,
            "VIRTUAL_CONTAINER_E2E_SCOPE": "auto",
        }
        if inputs.backend == "cloud-hypervisor":
            prefix = "EXECUTION_ISOLATION_CLOUD_HYPERVISOR_E2E_"
            environment.update(
                {
                    f"{prefix}BACKEND": inputs.backend,
                    f"{prefix}ARTIFACT_MANIFEST": str(manifest),
                    f"{prefix}CTR_RUNTIME": inputs.runtime,
                    f"{prefix}IMAGE": inputs.workload_image,
                    f"{prefix}IMAGE_PULL_POLICY": inputs.image_pull_policy,
                }
            )
        if inputs.workload_image_archive is not None:
            archive = str(inputs.workload_image_archive)
            environment["VIRTUAL_CONTAINER_E2E_IMAGE_ARCHIVE"] = archive
            if inputs.backend == "cloud-hypervisor":
                environment[
                    "EXECUTION_ISOLATION_CLOUD_HYPERVISOR_E2E_IMAGE_ARCHIVE"
                ] = archive
        return environment

    @classmethod
    def _path_prepend(cls, inputs: ProfileInputs) -> list[str]:
        paths = [
            "${REPO}/local/kata/bin",
            str(inputs.hypervisor.parent),
            str(inputs.virtiofsd.parent),
            *cls._invoking_user_bin_dirs(),
            "/opt/kata/bin",
            "/usr/local/bin",
            "/usr/local/sbin",
            "/usr/sbin",
            "/usr/bin",
            "/bin",
        ]
        return list(dict.fromkeys(paths))

    @staticmethod
    def _invoking_user_bin_dirs() -> list[str]:
        user = os.environ.get("SUDO_USER")
        if not user or user == "root":
            return []
        try:
            home = Path(pwd.getpwnam(user).pw_dir)
        except KeyError:
            return []
        return [str(home / ".local/bin"), str(home / ".cargo/bin")]
