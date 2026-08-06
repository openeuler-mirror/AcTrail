from __future__ import annotations

import hashlib
import json
import re
from dataclasses import dataclass, field
from pathlib import Path, PurePosixPath
from typing import Any

from .checksums import sha256_file
from .process import CommandRunner
from .requirements import ArtifactRequirement, PreparePolicy
from .runtime_config import load_hypervisor_table


_MANIFEST_LINE = re.compile(r"^([0-9a-fA-F]{64})[ \t]+(.+)$")
_SHA256 = re.compile(r"^[0-9a-f]{64}$")
_RELEASE_LAYOUT = {
    "actraild_sha256": "actraild",
    "actrailctl_sha256": "actrailctl",
    "actrailviewer_sha256": "actrailviewer",
    "tls_probe_sha256": "libactrail_tls_payload_probe_sync.so",
}


@dataclass(frozen=True)
class DeploymentArtifacts:
    """Validated content-addressed inputs for the Kata V2 acceptance tests."""

    manifest: Path
    cache_key: str
    source_commit: str
    backend: str
    runtime: str
    guest_bundle: Path
    workload_bundle: Path
    base_image: Path
    data_image: Path
    base_config: Path
    data_config: Path
    workload_image: str
    xiaoo: Path | None

    @classmethod
    def load(
        cls,
        manifest: Path,
        *,
        bin_dir: Path,
        expected_backend: str,
        expected_runtime: str,
        require_xiaoo: bool = False,
    ) -> DeploymentArtifacts:
        if not manifest.is_absolute():
            raise ValueError(f"artifact manifest path must be absolute: {manifest}")
        try:
            manifest = manifest.resolve(strict=True)
        except OSError as error:
            raise RuntimeError(f"artifact manifest is missing: {manifest}") from error
        if not manifest.is_file():
            raise RuntimeError(f"artifact manifest is not a file: {manifest}")
        root = manifest.parent.resolve()
        document = _read_json_object(manifest, "artifact manifest")
        if document.get("format") != 2:
            raise RuntimeError("artifact manifest format must be 2")

        cache_key = _required_sha256(document, "cache_key")
        if root.name != cache_key:
            raise RuntimeError(
                "artifact digest directory does not match manifest cache_key: "
                f"directory={root.name} cache_key={cache_key}"
            )
        source_commit = _required_string(document, "source_commit")
        backend = _required_string(document, "backend")
        runtime = _required_string(document, "runtime")
        if backend != expected_backend:
            raise RuntimeError(
                "artifact backend mismatch: "
                f"expected={expected_backend} actual={backend}"
            )
        if runtime != expected_runtime:
            raise RuntimeError(
                "artifact runtime mismatch: "
                f"expected={expected_runtime} actual={runtime}"
            )

        release = _required_object(document, "release")
        for digest_name, filename in _RELEASE_LAYOUT.items():
            expected = _required_sha256(release, digest_name)
            _verify_file_digest(
                bin_dir / filename,
                expected,
                f"current release {filename}",
            )

        guest = _required_object(document, "guest_bundle")
        guest_bundle = _artifact_path(
            root,
            _required_string(guest, "path"),
            kind="directory",
        )
        guest_manifest = guest_bundle / "MANIFEST.sha256"
        _verify_file_digest(
            guest_manifest,
            _required_sha256(guest, "manifest_sha256"),
            "guest bundle manifest",
        )
        _verify_manifest(guest_bundle, guest_manifest)
        for digest_name, filename in _RELEASE_LAYOUT.items():
            _verify_file_digest(
                guest_bundle / filename,
                _required_sha256(release, digest_name),
                f"guest bundle {filename}",
            )

        workload = _required_object(document, "workload_bundle")
        workload_bundle = _artifact_path(
            root,
            _required_string(workload, "path"),
            kind="directory",
        )
        workload_manifest = workload_bundle / "MANIFEST.sha256"
        _verify_file_digest(
            workload_manifest,
            _required_sha256(workload, "manifest_sha256"),
            "workload bundle manifest",
        )
        _verify_manifest(workload_bundle, workload_manifest)
        workload_actrailctl = workload_bundle / "bin/actrailctl"
        workload_actrailctl_sha256 = _required_sha256(
            workload,
            "actrailctl_sha256",
        )
        _verify_file_digest(
            workload_actrailctl,
            workload_actrailctl_sha256,
            "workload bundle actrailctl",
        )
        if workload_actrailctl_sha256 != _required_sha256(
            release,
            "actrailctl_sha256",
        ):
            raise RuntimeError(
                "workload bundle actrailctl does not match the current release"
            )

        images = _required_object(document, "images")
        base_image = _artifact_path(
            root,
            _required_string(images, "base"),
            kind="file",
        )
        data_image = _artifact_path(
            root,
            _required_string(images, "data"),
            kind="file",
        )
        configs = _required_object(document, "runtime_configs")
        base_config = _artifact_path(
            root,
            _required_string(configs, "base"),
            kind="file",
        )
        data_config = _artifact_path(
            root,
            _required_string(configs, "data"),
            kind="file",
        )
        integrity = _required_object(document, "integrity")
        for config in (base_config, data_config):
            relative = config.relative_to(root).as_posix()
            _verify_file_digest(
                config,
                _required_sha256(integrity, relative),
                f"runtime config {relative}",
            )
        inputs = _required_object(document, "inputs")
        if inputs.get("format") != 1:
            raise RuntimeError("artifact manifest inputs format must be 1")
        input_files = _required_object(inputs, "files")
        input_paths = _required_object(inputs, "paths")
        runtime_inputs = {
            name: _verified_external_input(input_files, input_paths, name)
            for name in (
                "hypervisor",
                "base_kernel",
                "data_kernel",
                "virtiofsd",
            )
        }
        _verify_runtime_config_assets(
            base_config,
            backend=backend,
            profile="base",
            expected_image=base_image,
            expected_hypervisor=runtime_inputs["hypervisor"],
            expected_kernel=runtime_inputs["base_kernel"],
            expected_virtiofsd=runtime_inputs["virtiofsd"],
        )
        _verify_runtime_config_assets(
            data_config,
            backend=backend,
            profile="data",
            expected_image=data_image,
            expected_hypervisor=runtime_inputs["hypervisor"],
            expected_kernel=runtime_inputs["data_kernel"],
            expected_virtiofsd=runtime_inputs["virtiofsd"],
        )

        workload_image_document = _required_object(document, "workload_image")
        workload_image = _required_string(workload_image_document, "reference")

        xiaoo_document = document.get("xiaoo")
        xiaoo = None
        if xiaoo_document is not None:
            xiaoo_object = _object(xiaoo_document, "xiaoo")
            xiaoo = _artifact_path(
                root,
                _required_string(xiaoo_object, "path"),
                kind="file",
            )
            _verify_file_digest(
                xiaoo,
                _required_sha256(xiaoo_object, "sha256"),
                "xiaoO executable",
            )
            if not xiaoo.stat().st_mode & 0o111:
                raise RuntimeError(f"xiaoO artifact is not executable: {xiaoo}")
        if require_xiaoo and xiaoo is None:
            raise RuntimeError(
                "artifact manifest does not contain the xiaoO executable"
            )

        return cls(
            manifest=manifest,
            cache_key=cache_key,
            source_commit=source_commit,
            backend=backend,
            runtime=runtime,
            guest_bundle=guest_bundle,
            workload_bundle=workload_bundle,
            base_image=base_image,
            data_image=data_image,
            base_config=base_config,
            data_config=data_config,
            workload_image=workload_image,
            xiaoo=xiaoo,
        )


def resolve_deployment_artifacts(
    manifest: Path | None,
    *,
    bin_dir: Path,
    guest_bundle: Path,
    workload_bundle: Path,
    expected_backend: str,
    expected_runtime: str,
    expected_workload_image: str,
) -> DeploymentArtifacts | None:
    """Resolve either a format-2 manifest or the deprecated mutable bundles."""

    if manifest is None:
        validate_release_bundle_consistency(
            bin_dir,
            guest_bundle,
            workload_bundle,
        )
        return None
    try:
        deployment = DeploymentArtifacts.load(
            manifest,
            bin_dir=bin_dir,
            expected_backend=expected_backend,
            expected_runtime=expected_runtime,
        )
    except (OSError, RuntimeError, ValueError) as error:
        raise RuntimeError(str(error)) from error
    if deployment.workload_image != expected_workload_image:
        raise RuntimeError(
            "workload image mismatch: "
            f"profile={expected_workload_image} "
            f"manifest={deployment.workload_image}"
        )
    return deployment


def validate_release_bundle_consistency(
    bin_dir: Path,
    guest_bundle: Path,
    workload_bundle: Path,
) -> None:
    """Validate deprecated mutable bundle paths against the current release."""

    DirectoryManifestRequirement(guest_bundle).ensure(PreparePolicy.CHECK_ONLY)
    DirectoryManifestRequirement(workload_bundle).ensure(PreparePolicy.CHECK_ONLY)
    for filename in _RELEASE_LAYOUT.values():
        release = bin_dir / filename
        if not release.is_file():
            raise RuntimeError(f"current release artifact is missing: {release}")
        _verify_file_digest(
            guest_bundle / filename,
            sha256_file(release),
            f"guest bundle {filename}",
        )
    _verify_file_digest(
        workload_bundle / "bin/actrailctl",
        sha256_file(bin_dir / "actrailctl"),
        "workload bundle actrailctl",
    )


@dataclass
class DirectoryManifestRequirement:
    directory: Path
    manifest_name: str = "MANIFEST.sha256"
    prepare_command: tuple[str, ...] | None = None
    runner: CommandRunner | None = None
    timeout_seconds: float = 600
    _digest: str | None = field(default=None, init=False, repr=False)

    def __post_init__(self) -> None:
        if not self.directory.is_absolute():
            raise ValueError(
                f"artifact directory must be absolute: {self.directory}"
            )
        manifest_path = PurePosixPath(self.manifest_name)
        if manifest_path.is_absolute() or ".." in manifest_path.parts:
            raise ValueError(f"unsafe artifact manifest name: {self.manifest_name}")
        if self.prepare_command is not None and (
            not self.prepare_command or any(not item for item in self.prepare_command)
        ):
            raise ValueError("artifact prepare command must contain non-empty argv")
        if (self.prepare_command is None) != (self.runner is None):
            raise ValueError(
                "artifact prepare command and command runner must be "
                "configured together"
            )
        if self.timeout_seconds <= 0:
            raise ValueError("artifact preparation timeout must be positive")

    @property
    def digest(self) -> str:
        if self._digest is None:
            raise RuntimeError("artifact requirement has not been ensured")
        return self._digest

    def ensure(self, policy: PreparePolicy) -> None:
        manifest = self.directory / self.manifest_name
        if not manifest.is_file():
            if policy is PreparePolicy.CHECK_ONLY or self.prepare_command is None:
                raise FileNotFoundError(f"artifact manifest is missing: {manifest}")
            assert self.runner is not None
            result = self.runner.run(
                self.prepare_command,
                timeout=self.timeout_seconds,
            )
            if result.returncode != 0:
                raise RuntimeError(
                    f"failed to prepare artifact exit={result.returncode}: "
                    f"{result.diagnostic or 'no diagnostic output'}"
                )
            if not manifest.is_file():
                raise RuntimeError(
                    "artifact preparation completed without producing manifest: "
                    f"{manifest}"
                )
        self._digest = _verify_manifest(self.directory, manifest)


@dataclass(frozen=True)
class CompositeArtifactRequirement:
    requirements: tuple[ArtifactRequirement, ...]

    def ensure(self, policy: PreparePolicy) -> None:
        for requirement in self.requirements:
            requirement.ensure(policy)


def _verify_manifest(directory: Path, manifest: Path) -> str:
    try:
        content = manifest.read_bytes()
        lines = content.decode("utf-8").splitlines()
    except (OSError, UnicodeDecodeError) as error:
        raise RuntimeError(
            f"cannot read artifact manifest {manifest}: {error}"
        ) from error
    if not lines:
        raise RuntimeError(f"artifact manifest is empty: {manifest}")
    root = directory.resolve()
    for line_number, line in enumerate(lines, start=1):
        match = _MANIFEST_LINE.fullmatch(line)
        if match is None:
            raise RuntimeError(
                f"invalid artifact manifest line {line_number}: {manifest}"
            )
        expected, raw_path = match.groups()
        raw_path = raw_path.removeprefix("*")
        relative = PurePosixPath(raw_path)
        if relative.parts[:1] == (".",):
            relative = PurePosixPath(*relative.parts[1:])
        if relative.is_absolute() or not relative.parts or ".." in relative.parts:
            raise RuntimeError(
                f"unsafe artifact path on line {line_number}: {raw_path}"
            )
        candidate = directory.joinpath(*relative.parts)
        try:
            resolved = candidate.resolve(strict=True)
        except OSError as error:
            raise RuntimeError(
                f"artifact listed by manifest is missing: {candidate}"
            ) from error
        if root != resolved and root not in resolved.parents:
            raise RuntimeError(f"artifact path escapes bundle: {candidate}")
        if not resolved.is_file():
            raise RuntimeError(f"artifact path is not a file: {candidate}")
        actual = sha256_file(resolved)
        if actual.lower() != expected.lower():
            raise RuntimeError(
                f"artifact checksum mismatch: {candidate} "
                f"expected={expected} actual={actual}"
            )
    return hashlib.sha256(content).hexdigest()


def _read_json_object(path: Path, name: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise RuntimeError(f"cannot read {name} {path}: {error}") from error
    return _object(value, name)


def _object(value: Any, name: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise RuntimeError(f"{name} must be a JSON object")
    return value


def _required_object(document: dict[str, Any], name: str) -> dict[str, Any]:
    if name not in document:
        raise RuntimeError(f"artifact manifest is missing object: {name}")
    return _object(document[name], name)


def _required_string(document: dict[str, Any], name: str) -> str:
    value = document.get(name)
    if not isinstance(value, str) or not value:
        raise RuntimeError(f"artifact manifest field {name} must be a string")
    return value


def _required_sha256(document: dict[str, Any], name: str) -> str:
    value = _required_string(document, name).lower()
    if _SHA256.fullmatch(value) is None:
        raise RuntimeError(
            f"artifact manifest field {name} must be a SHA-256 digest"
        )
    return value


def _artifact_path(root: Path, raw_path: str, *, kind: str) -> Path:
    relative = PurePosixPath(raw_path)
    if relative.is_absolute() or not relative.parts or ".." in relative.parts:
        raise RuntimeError(f"unsafe artifact path: {raw_path}")
    candidate = root.joinpath(*relative.parts)
    try:
        resolved = candidate.resolve(strict=True)
    except OSError as error:
        raise RuntimeError(f"artifact path is missing: {candidate}") from error
    if resolved != root and root not in resolved.parents:
        raise RuntimeError(f"unsafe artifact path escapes digest directory: {raw_path}")
    if kind == "file" and not resolved.is_file():
        raise RuntimeError(f"artifact path is not a file: {resolved}")
    if kind == "directory" and not resolved.is_dir():
        raise RuntimeError(f"artifact path is not a directory: {resolved}")
    return resolved


def _verify_file_digest(path: Path, expected: str, label: str) -> None:
    if not path.is_file():
        raise RuntimeError(f"{label} is missing: {path}")
    actual = sha256_file(path)
    if actual != expected:
        raise RuntimeError(
            f"{label} checksum mismatch: expected={expected} actual={actual}"
        )


def _verify_runtime_config_assets(
    config: Path,
    *,
    backend: str,
    profile: str,
    expected_image: Path,
    expected_hypervisor: Path,
    expected_kernel: Path,
    expected_virtiofsd: Path,
) -> None:
    try:
        settings = load_hypervisor_table(config, backend)
    except (OSError, KeyError, TypeError, ValueError) as error:
        raise RuntimeError(f"cannot read runtime config {config}: {error}") from error
    image = _absolute_runtime_path(config, settings, "image")
    if image != expected_image.resolve():
        raise RuntimeError(
            f"runtime config {config} does not reference its manifest image: "
            f"configured={image} expected={expected_image}"
        )
    hypervisor = _absolute_runtime_path(config, settings, "path")
    kernel = _absolute_runtime_path(config, settings, "kernel")
    virtiofsd = _absolute_runtime_path(config, settings, "virtio_fs_daemon")
    _verify_expected_runtime_path(config, "path", hypervisor, expected_hypervisor)
    _verify_executable(hypervisor, "runtime hypervisor")
    _verify_expected_runtime_path(
        config,
        f"{profile} kernel",
        kernel,
        expected_kernel,
    )
    _verify_expected_runtime_path(
        config,
        "virtio_fs_daemon",
        virtiofsd,
        expected_virtiofsd,
    )
    _verify_executable(virtiofsd, "runtime virtiofsd")


def _verified_external_input(
    input_files: dict[str, Any],
    input_paths: dict[str, Any],
    name: str,
) -> Path:
    expected = _required_sha256(input_files, name)
    path = Path(_required_string(input_paths, name)).expanduser()
    if not path.is_absolute():
        raise RuntimeError(f"artifact input path must be absolute: {name}={path}")
    _verify_file_digest(path, expected, f"runtime input {name}")
    return path.resolve()


def _verify_expected_runtime_path(
    config: Path,
    name: str,
    configured: Path,
    expected: Path,
) -> None:
    if configured != expected:
        raise RuntimeError(
            f"runtime config {config} does not reference manifest input {name}: "
            f"configured={configured} expected={expected}"
        )


def _absolute_runtime_path(
    config: Path,
    settings: dict[str, Any],
    name: str,
) -> Path:
    raw_path = settings.get(name)
    if not isinstance(raw_path, str) or not raw_path:
        raise RuntimeError(
            f"runtime config does not define {name} in its hypervisor section: "
            f"{config}"
        )
    path = Path(raw_path)
    if not path.is_absolute():
        raise RuntimeError(
            f"runtime config {config} defines a non-absolute {name}: {raw_path}"
        )
    return path.resolve(strict=False)


def _verify_executable(path: Path, label: str) -> None:
    if not path.stat().st_mode & 0o111:
        raise RuntimeError(f"{label} is not executable: {path}")
