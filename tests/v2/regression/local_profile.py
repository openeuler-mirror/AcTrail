"""Load optional machine-local defaults for the V2 regression runner."""

from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any


DEFAULT_PROFILE = Path("local/kata/v2-test-profile.json")

_ALLOWED_ENVIRONMENT = {
    "ACTRAIL_BIN_DIR",
    "ACTRAIL_VMM_BIN_DIR",
    "BACKEND",
    "CAPABILITY_BUNDLE_DIR",
    "CTR_RUNTIME",
    "KATA_CONFIG_DIRS",
    "WORKLOAD_BUNDLE_DIR",
    "XIAOO_E2E_BINARY",
    "VIRTUAL_CONTAINER_E2E_ARTIFACT_MANIFEST",
    "VIRTUAL_CONTAINER_E2E_BACKENDS",
    "VIRTUAL_CONTAINER_E2E_IMAGE",
    "VIRTUAL_CONTAINER_E2E_IMAGE_ARCHIVE",
    "VIRTUAL_CONTAINER_E2E_IMAGE_PULL_POLICY",
    "VIRTUAL_CONTAINER_E2E_PULL_IMAGE",
    "VIRTUAL_CONTAINER_E2E_RUNTIME_TIMEOUT_SECONDS",
    "VIRTUAL_CONTAINER_E2E_SCOPE",
    "VIRTUAL_CONTAINER_E2E_SETTLE_SECONDS",
    "VIRTUAL_CONTAINER_E2E_STRATOVIRT_CONFIG",
    "VIRTUAL_CONTAINER_E2E_STRATOVIRT_DATA_CONFIG",
    "VIRTUAL_CONTAINER_E2E_CLOUD_HYPERVISOR_CONFIG",
    "VIRTUAL_CONTAINER_E2E_CLOUD_HYPERVISOR_DATA_CONFIG",
    "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_BACKEND",
    "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_ARTIFACT_MANIFEST",
    "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_CTR_RUNTIME",
    "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_IMAGE",
    "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_IMAGE_ARCHIVE",
    "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_IMAGE_PULL_POLICY",
    "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_OVERLAP_TIMEOUT_SECONDS",
    "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_READY_TIMEOUT_SECONDS",
    "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_RUNTIME_CONFIG",
    "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_RUNTIME_TIMEOUT_SECONDS",
    "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_WORKLOAD_BUNDLE",
    "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_XIAOO_BINARY",
}


def _expand(value: str, repo: Path, profile_dir: Path) -> str:
    return value.replace("${REPO}", str(repo)).replace(
        "${PROFILE_DIR}", str(profile_dir)
    )


def _object(value: Any, name: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ValueError(f"V2 test profile {name} must be an object")
    return value


def load_local_test_profile(
    repo: Path,
    explicit_path: Path | None = None,
) -> Path | None:
    """Apply a local profile without replacing explicit shell overrides."""

    repo = repo.resolve()
    configured_path = os.environ.get("ACTRAIL_TEST_PROFILE")
    required = explicit_path is not None or configured_path is not None
    if explicit_path is not None:
        profile_path = explicit_path.expanduser()
    elif configured_path:
        profile_path = Path(configured_path).expanduser()
    else:
        profile_path = repo / DEFAULT_PROFILE
    if not profile_path.is_absolute():
        profile_path = (Path.cwd() / profile_path).resolve()
    else:
        profile_path = profile_path.resolve()

    if not profile_path.is_file():
        if required:
            raise ValueError(f"V2 test profile does not exist: {profile_path}")
        return None

    try:
        document = json.loads(profile_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(
            f"cannot read V2 test profile {profile_path}: {error}"
        ) from error
    profile = _object(document, "root")
    if profile.get("format") not in {1, 2}:
        raise ValueError("V2 test profile format must be 1 or 2")

    profile_dir = profile_path.parent
    environment = _object(profile.get("environment", {}), "environment")
    unknown = sorted(set(environment) - _ALLOWED_ENVIRONMENT)
    if unknown:
        raise ValueError(
            "V2 test profile contains unsupported environment key(s): "
            + ", ".join(unknown)
        )
    for name, raw_value in environment.items():
        if not isinstance(raw_value, str):
            raise ValueError(
                f"V2 test profile environment value {name} must be a string"
            )
        if name not in os.environ:
            os.environ[name] = _expand(raw_value, repo, profile_dir)

    raw_path_prepend = profile.get("path_prepend", [])
    if not isinstance(raw_path_prepend, list) or not all(
        isinstance(item, str) for item in raw_path_prepend
    ):
        raise ValueError("V2 test profile path_prepend must be a string array")
    path_entries = [
        _expand(item, repo, profile_dir) for item in raw_path_prepend
    ]
    path_entries.extend(os.environ.get("PATH", "").split(os.pathsep))
    os.environ["PATH"] = os.pathsep.join(
        dict.fromkeys(item for item in path_entries if item)
    )
    return profile_path
