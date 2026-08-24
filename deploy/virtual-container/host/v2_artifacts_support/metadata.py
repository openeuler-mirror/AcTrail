from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
from pathlib import Path
from typing import Any

from tests.v2.common.kata_runtime import runtime_path, sha256_file

from .model import RELEASE_FILES, PreparationInputs


def build_input_document(inputs: PreparationInputs) -> dict[str, Any]:
    inputs.validate()
    files: dict[str, str] = {}
    named_inputs = {
        "base_config_source": inputs.base_config_source,
        "data_config_source": inputs.data_config_source,
        "base_image_source": inputs.base_image_source,
        "data_image_source": inputs.data_image_source,
        "hypervisor": inputs.hypervisor,
        "base_kernel": inputs.base_kernel,
        "data_kernel": inputs.data_kernel,
        "virtiofsd": inputs.virtiofsd,
    }
    if inputs.xiaoo is not None:
        named_inputs["xiaoo"] = inputs.xiaoo
    if inputs.workload_image_archive is not None:
        named_inputs["workload_image_archive"] = inputs.workload_image_archive
    for name, path in sorted(named_inputs.items()):
        files[name] = sha256_file(path)
    for digest_name, filename in RELEASE_FILES.items():
        files[f"release.{digest_name}"] = sha256_file(inputs.bin_dir / filename)
    for path in sorted(inputs.tool_inputs, key=lambda item: str(item)):
        if path.is_file():
            relative = display_path(path, inputs.repo)
            files[f"tool.{relative}"] = sha256_file(path)
    return {
        "format": 1,
        "backend": inputs.backend,
        "runtime": inputs.runtime,
        "otel_endpoint": inputs.otel_endpoint,
        "egress_mode": inputs.egress_mode,
        "socket_gid": inputs.socket_gid,
        "data_vcpus": inputs.data_vcpus,
        "workload_image": inputs.workload_image,
        "image_pull_policy": inputs.image_pull_policy,
        "files": files,
        "paths": {
            name: str(path.resolve())
            for name, path in sorted(named_inputs.items())
        },
    }


def cache_key_for(input_document: dict[str, Any]) -> str:
    encoded = json.dumps(
        input_document,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def infer_runtime_path(config: Path, backend: str, key: str) -> Path:
    try:
        return runtime_path(config, backend, key)
    except (OSError, KeyError, TypeError, ValueError) as error:
        raise ValueError(f"cannot read runtime config {config}: {error}") from error


def default_tool_inputs(repo: Path) -> tuple[Path, ...]:
    roots = (
        repo / "deploy/virtual-container/guest",
        repo / "deploy/virtual-container/workload",
        repo / "deploy/virtual-container/host/argparse_types.py",
        repo / "deploy/virtual-container/host/prepare-stratovirt-config.py",
        repo / "deploy/virtual-container/host/prepare-v2-test-artifacts.py",
        repo / "deploy/virtual-container/host/v2_artifacts.py",
        repo / "deploy/virtual-container/host/v2_artifacts_support",
        repo
        / "tests/v2/regression/virtual_container/prepare-guest-bundle.sh",
        repo / "tests/v2/regression/execution_isolation_cloud_hypervisor",
    )
    files: list[Path] = []
    for root in roots:
        if root.is_file():
            files.append(root)
        elif root.is_dir():
            files.extend(
                path
                for path in root.rglob("*")
                if path.is_file()
                and "__pycache__" not in path.parts
                and path.suffix != ".pyc"
            )
    return tuple(files)


def release_hashes(bin_dir: Path) -> dict[str, str]:
    return {
        digest_name: sha256_file(bin_dir / filename)
        for digest_name, filename in RELEASE_FILES.items()
    }


def source_commit(repo: Path) -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), "rev-parse", "HEAD"],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return result.stdout.strip() if result.returncode == 0 else "unknown"


def restore_invoking_user_ownership(
    output_root: Path,
    manifest: Path,
    profile_path: Path | None,
) -> None:
    if os.geteuid() != 0:
        return
    try:
        uid = int(os.environ["SUDO_UID"])
        gid = int(os.environ["SUDO_GID"])
    except (KeyError, ValueError):
        return
    if uid == 0:
        return
    chown_tree(output_root, uid, gid, recursive=False)
    chown_tree(manifest.parent, uid, gid, recursive=True)
    if profile_path is not None:
        chown_tree(profile_path, uid, gid, recursive=False)


def chown_tree(path: Path, uid: int, gid: int, *, recursive: bool) -> None:
    if not path.exists() and not path.is_symlink():
        return
    if recursive and path.is_dir():
        for directory, directory_names, filenames in os.walk(
            path,
            topdown=False,
            followlinks=False,
        ):
            parent = Path(directory)
            for name in filenames:
                os.chown(parent / name, uid, gid, follow_symlinks=False)
            for name in directory_names:
                os.chown(parent / name, uid, gid, follow_symlinks=False)
    os.chown(path, uid, gid, follow_symlinks=False)


def fsync_tree(directory: Path) -> None:
    for path in directory.rglob("*"):
        if path.is_file():
            with path.open("rb") as source:
                os.fsync(source.fileno())
    descriptor = os.open(directory, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def shell_display(value: str) -> str:
    if re.fullmatch(r"[A-Za-z0-9_./:=+,-]+", value):
        return value
    return json.dumps(value)


def display_path(path: Path, repo: Path) -> str:
    try:
        return path.resolve().relative_to(repo.resolve()).as_posix()
    except ValueError:
        return str(path.resolve())
