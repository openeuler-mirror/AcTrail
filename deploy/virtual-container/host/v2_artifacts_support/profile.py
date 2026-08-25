"""Machine-local profile writer for V2 virtual-container tests."""

from __future__ import annotations

import json
import os
import pwd
from pathlib import Path, PurePosixPath
from typing import Protocol

from .io import atomic_json


class ProfileInputs(Protocol):
    backend: str
    bin_dir: Path
    hypervisor: Path
    image_pull_policy: str
    runtime: str
    virtiofsd: Path | None
    workload_image: str
    workload_image_archive: Path | None


class V2TestProfile:
    _BACKEND_NAMES = {
        "stratovirt": "StratoVirt",
        "cloud-hypervisor": "Cloud Hypervisor",
        "firecracker": "Firecracker",
    }
    _MANIFEST_ENVIRONMENT_KEYS = {
        "stratovirt": "VIRTUAL_CONTAINER_E2E_ARTIFACT_MANIFEST",
        "cloud-hypervisor": "VIRTUAL_CONTAINER_E2E_ARTIFACT_MANIFEST",
        "firecracker": (
            "EXECUTION_ISOLATION_FIRECRACKER_E2E_ARTIFACT_MANIFEST"
        ),
    }
    _EXECUTION_PREFIXES = {
        "stratovirt": "EXECUTION_ISOLATION_STRATOVIRT_E2E_",
        "cloud-hypervisor": "EXECUTION_ISOLATION_CLOUD_HYPERVISOR_E2E_",
        "firecracker": "EXECUTION_ISOLATION_FIRECRACKER_E2E_",
    }

    @classmethod
    def write(
        cls,
        inputs: ProfileInputs,
        manifest: Path,
        profile_path: Path,
        *,
        workload_image: str | None = None,
        workload_image_archive: Path | None = None,
    ) -> None:
        profile = {
            "format": 2,
            "name": (
                "openEuler / Kata 3.32 / "
                + cls._BACKEND_NAMES[inputs.backend]
            ),
            "path_prepend": cls._path_prepend(inputs),
            "environment": cls._environment(
                inputs,
                manifest,
                workload_image=workload_image,
                workload_image_archive=workload_image_archive,
            ),
        }
        profile_path.parent.mkdir(parents=True, exist_ok=True)
        atomic_json(profile_path, profile)
        profile_path.chmod(0o644)

    @classmethod
    def validate(cls, profile_path: Path, manifest: Path) -> None:
        try:
            manifest_document = json.loads(manifest.read_text(encoding="utf-8"))
            backend = manifest_document["backend"]
            manifest_key = cls._MANIFEST_ENVIRONMENT_KEYS[backend]
            workload = manifest_document["workload_image"]
            expected_image = workload["reference"]
            document = json.loads(profile_path.read_text(encoding="utf-8"))
            environment = document["environment"]
            configured = environment[manifest_key]
            execution_prefix = cls._EXECUTION_PREFIXES[backend]
            configured_image = environment[f"{execution_prefix}IMAGE"]
        except (OSError, KeyError, TypeError, json.JSONDecodeError) as error:
            raise RuntimeError(
                f"cannot validate V2 test profile {profile_path}"
            ) from error
        if configured != str(manifest):
            raise RuntimeError(
                f"V2 test profile {manifest_key} points at {configured}, "
                f"expected {manifest}"
            )
        if configured_image != expected_image:
            raise RuntimeError(
                "V2 test profile workload image points at "
                f"{configured_image}, expected {expected_image}"
            )
        archive_value = workload.get("archive")
        if archive_value is None:
            return
        if not isinstance(archive_value, str):
            raise RuntimeError(
                "artifact workload image archive path is invalid"
            )
        relative = PurePosixPath(archive_value)
        if relative.is_absolute() or not relative.parts or ".." in relative.parts:
            raise RuntimeError(
                "artifact workload image archive path is unsafe"
            )
        archive_key = f"{execution_prefix}IMAGE_ARCHIVE"
        configured_archive = environment.get(archive_key)
        expected_archive = manifest.parent.joinpath(*relative.parts)
        if configured_archive != str(expected_archive):
            raise RuntimeError(
                "V2 test profile workload image archive points at "
                f"{configured_archive}, expected {expected_archive}"
            )

    @staticmethod
    def _environment(
        inputs: ProfileInputs,
        manifest: Path,
        *,
        workload_image: str | None = None,
        workload_image_archive: Path | None = None,
    ) -> dict[str, str]:
        if workload_image is None:
            workload_image = inputs.workload_image
        if workload_image_archive is None:
            workload_image_archive = inputs.workload_image_archive
        environment = {
            "ACTRAIL_BIN_DIR": str(inputs.bin_dir),
            "CTR_RUNTIME": inputs.runtime,
        }
        if inputs.backend == "firecracker":
            environment.update(
                {
                    "VIRTUAL_CONTAINER_E2E_BACKENDS": "stratovirt",
                    "VIRTUAL_CONTAINER_E2E_SCOPE": "contracts",
                }
            )
        else:
            environment.update(
                {
                    "VIRTUAL_CONTAINER_E2E_BACKENDS": inputs.backend,
                    "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_BACKEND": (
                        inputs.backend
                    ),
                    "VIRTUAL_CONTAINER_E2E_ARTIFACT_MANIFEST": str(manifest),
                    "VIRTUAL_CONTAINER_E2E_IMAGE": workload_image,
                    "VIRTUAL_CONTAINER_E2E_IMAGE_PULL_POLICY": (
                        inputs.image_pull_policy
                    ),
                    "VIRTUAL_CONTAINER_E2E_SCOPE": "auto",
                }
            )
        execution_prefix = V2TestProfile._EXECUTION_PREFIXES[inputs.backend]
        environment.update(
            {
                f"{execution_prefix}BACKEND": inputs.backend,
                f"{execution_prefix}ARTIFACT_MANIFEST": str(manifest),
                f"{execution_prefix}CTR_RUNTIME": inputs.runtime,
                f"{execution_prefix}IMAGE": workload_image,
                f"{execution_prefix}IMAGE_PULL_POLICY": (
                    inputs.image_pull_policy
                ),
            }
        )
        if workload_image_archive is not None:
            archive = str(workload_image_archive)
            if inputs.backend != "firecracker":
                environment["VIRTUAL_CONTAINER_E2E_IMAGE_ARCHIVE"] = archive
            environment[f"{execution_prefix}IMAGE_ARCHIVE"] = archive
        return environment

    @classmethod
    def _path_prepend(cls, inputs: ProfileInputs) -> list[str]:
        paths = [
            "${REPO}/local/kata/bin",
            str(inputs.hypervisor.parent),
        ]
        if inputs.virtiofsd is not None:
            paths.append(str(inputs.virtiofsd.parent))
        paths.extend(
            [
                *cls._invoking_user_bin_dirs(),
                "/opt/kata/bin",
                "/usr/local/bin",
                "/usr/local/sbin",
                "/usr/sbin",
                "/usr/bin",
                "/bin",
            ]
        )
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
