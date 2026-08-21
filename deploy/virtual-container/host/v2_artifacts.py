"""Content-addressed deployment preparation for Kata V2 acceptance tests."""

from __future__ import annotations

import hashlib
import json
import os
import pwd
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Sequence


REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT))

from tests.v2.common.kata_runtime import (  # noqa: E402
    DeploymentArtifacts,
    runtime_path,
    sha256_file,
)


RELEASE_FILES = {
    "actraild_sha256": "actraild",
    "actrailctl_sha256": "actrailctl",
    "actrailviewer_sha256": "actrailviewer",
    "tls_probe_sha256": "libactrail_tls_payload_probe_sync.so",
}
SUPPORTED_BACKENDS = {"stratovirt", "cloud-hypervisor"}
BACKEND_DISPLAY_NAMES = {
    "stratovirt": "StratoVirt",
    "cloud-hypervisor": "Cloud Hypervisor",
}
SUPPORTED_PULL_POLICIES = {"never", "missing", "always"}
SUPPORTED_EGRESS_MODES = {"network", "vsock-bridge"}


@dataclass(frozen=True)
class PreparationInputs:
    repo: Path
    bin_dir: Path
    output_root: Path
    backend: str
    runtime: str
    kata_prefix: Path
    base_config_source: Path
    data_config_source: Path
    base_image_source: Path
    data_image_source: Path
    hypervisor: Path
    base_kernel: Path
    data_kernel: Path
    virtiofsd: Path
    xiaoo: Path | None
    workload_image: str
    workload_image_archive: Path | None
    image_pull_policy: str
    otel_endpoint: str | None
    socket_gid: int
    data_vcpus: int
    egress_mode: str = "network"
    tool_inputs: tuple[Path, ...] = ()

    def validate(self) -> None:
        if self.backend not in SUPPORTED_BACKENDS:
            raise ValueError(f"unsupported artifact backend: {self.backend}")
        if not self.runtime:
            raise ValueError("containerd runtime must not be empty")
        if self.image_pull_policy not in SUPPORTED_PULL_POLICIES:
            raise ValueError("image pull policy must be never, missing or always")
        if self.egress_mode not in SUPPORTED_EGRESS_MODES:
            raise ValueError("egress mode must be network or vsock-bridge")
        if self.otel_endpoint is not None and not self.otel_endpoint:
            raise ValueError("Guest OTLP/HTTP endpoint must be omitted or non-empty")
        if self.egress_mode == "vsock-bridge" and self.otel_endpoint is None:
            raise ValueError("vsock-bridge egress requires a Guest OTLP/HTTP endpoint")
        if not 1 <= self.socket_gid <= 2147483647:
            raise ValueError("socket GID must be between 1 and 2147483647")
        if self.data_vcpus < 2:
            raise ValueError("data vCPUs must be at least 2")
        for name, path in (
            ("repository", self.repo),
            ("release directory", self.bin_dir),
            ("output root parent", self.output_root.parent),
        ):
            if not path.is_absolute():
                raise ValueError(f"{name} path must be absolute: {path}")
        for name, path in (
            ("base config source", self.base_config_source),
            ("data config source", self.data_config_source),
            ("base image source", self.base_image_source),
            ("data image source", self.data_image_source),
            ("hypervisor", self.hypervisor),
            ("base kernel", self.base_kernel),
            ("data kernel", self.data_kernel),
            ("virtiofsd", self.virtiofsd),
        ):
            if not path.is_absolute() or not path.is_file():
                raise ValueError(f"{name} must be an existing absolute file: {path}")
        for name, path in (
            ("hypervisor", self.hypervisor),
            ("virtiofsd", self.virtiofsd),
        ):
            if not os.access(path, os.X_OK):
                raise ValueError(f"{name} must be executable: {path}")
        if self.xiaoo is not None:
            if not self.xiaoo.is_absolute() or not self.xiaoo.is_file():
                raise ValueError(
                    f"xiaoO must be an existing absolute file: {self.xiaoo}"
                )
            if not os.access(self.xiaoo, os.X_OK):
                raise ValueError(f"xiaoO must be executable: {self.xiaoo}")
        if self.workload_image_archive is not None and not (
            self.workload_image_archive.is_absolute()
            and self.workload_image_archive.is_file()
        ):
            raise ValueError(
                "workload image archive must be an existing absolute file: "
                f"{self.workload_image_archive}"
            )
        for filename in RELEASE_FILES.values():
            path = self.bin_dir / filename
            if not path.is_file():
                raise ValueError(f"release artifact is missing: {path}")


class CommandExecutor:
    def run(
        self,
        command: Sequence[str],
        *,
        environment: Mapping[str, str] | None = None,
        capture: bool = False,
    ) -> subprocess.CompletedProcess[str]:
        printable = " ".join(_shell_display(value) for value in command)
        print(f"+ {printable}", flush=True)
        result = subprocess.run(
            list(command),
            cwd=None,
            env=dict(environment) if environment is not None else None,
            text=True,
            stdout=subprocess.PIPE if capture else None,
            stderr=subprocess.PIPE if capture else None,
            check=False,
        )
        if result.returncode != 0:
            diagnostic = ((result.stderr or "") + (result.stdout or "")).strip()
            raise RuntimeError(
                f"command failed exit={result.returncode}: {printable}: "
                f"{diagnostic or 'no diagnostic output'}"
            )
        return result


class ArtifactPreparer:
    def __init__(
        self,
        inputs: PreparationInputs,
        executor: CommandExecutor | None = None,
    ) -> None:
        self.inputs = inputs
        self.executor = executor or CommandExecutor()

    def prepare(
        self,
        *,
        profile_path: Path | None,
        check_only: bool = False,
        ensure_workload_image: bool = True,
    ) -> Path:
        self.inputs.validate()
        print("artifact_inputs=hashing", flush=True)
        input_document = build_input_document(self.inputs)
        cache_key = cache_key_for(input_document)
        final = self.inputs.output_root / cache_key
        manifest = final / "manifest.json"
        if manifest.is_file():
            print(f"artifact_cache=hit digest={cache_key}", flush=True)
            self._validate_published(manifest)
        else:
            print(f"artifact_cache=miss digest={cache_key}", flush=True)
            if check_only:
                raise RuntimeError(
                    "content-addressed artifact is missing; rerun without "
                    f"--check-only: {manifest}"
                )
            manifest = self._build(final, cache_key, input_document)
        try:
            if ensure_workload_image:
                self._ensure_workload_image()
            if profile_path is not None:
                if check_only:
                    _check_profile(profile_path, manifest)
                else:
                    write_test_profile(self.inputs, manifest, profile_path)
        finally:
            _restore_invoking_user_ownership(
                self.inputs.output_root,
                manifest,
                profile_path,
            )
        print("ACTRAIL_V2_ARTIFACTS_READY", flush=True)
        print(f"artifact_digest={cache_key}", flush=True)
        print(f"artifact_manifest={manifest}", flush=True)
        if profile_path is not None:
            print(f"test_profile={profile_path}", flush=True)
        return manifest

    def _build(
        self,
        final: Path,
        cache_key: str,
        input_document: dict[str, Any],
    ) -> Path:
        output_root = self.inputs.output_root
        output_root.mkdir(parents=True, exist_ok=True)
        staging = Path(
            tempfile.mkdtemp(prefix=f".{cache_key}.", dir=output_root)
        )
        try:
            guest_bundle = staging / "guest-bundle"
            workload_bundle = staging / "workload-bundle"
            base_image = staging / "guest-base.img"
            data_image = staging / "guest-data.img"
            base_config = staging / "configuration-base.toml"
            data_config = staging / "configuration-data.toml"
            final_base_image = final / base_image.name
            final_data_image = final / data_image.name

            environment = os.environ.copy()
            environment.update(
                {
                    "ACTRAIL_REPO_ROOT": str(self.inputs.repo),
                    "ACTRAIL_BIN_DIR": str(self.inputs.bin_dir),
                    "BUNDLE_DIR": str(guest_bundle),
                    "ACTRAIL_BUILD": "0",
                }
            )
            self.executor.run(
                [
                    str(
                        self.inputs.repo
                        / "tests/v2/regression/virtual_container/"
                        "prepare-guest-bundle.sh"
                    )
                ],
                environment=environment,
            )
            self.executor.run(
                [
                    str(
                        self.inputs.repo
                        / "deploy/virtual-container/workload/prepare-bundle.sh"
                    ),
                    "--guest-bundle",
                    str(guest_bundle),
                    "--output",
                    str(workload_bundle),
                    "--socket-gid",
                    str(self.inputs.socket_gid),
                ],
                environment=environment,
            )
            self._inject(self.inputs.base_image_source, base_image, guest_bundle)
            self._inject(self.inputs.data_image_source, data_image, guest_bundle)
            self._make_config(
                self.inputs.base_config_source,
                self.inputs.base_kernel,
                base_image,
                final_base_image,
                base_config,
                data=False,
            )
            self._make_config(
                self.inputs.data_config_source,
                self.inputs.data_kernel,
                data_image,
                final_data_image,
                data_config,
                data=True,
            )
            if self.inputs.xiaoo is not None:
                shutil.copy2(self.inputs.xiaoo, staging / "xiaoo")
                (staging / "xiaoo").chmod(0o755)

            manifest_document = self._manifest_document(
                cache_key,
                input_document,
                staging,
            )
            _atomic_json(staging / "manifest.json", manifest_document)
            _fsync_tree(staging)
            try:
                os.replace(staging, final)
            except OSError:
                if not (final / "manifest.json").is_file():
                    raise
                shutil.rmtree(staging)
            manifest = final / "manifest.json"
            self._validate_published(manifest)
            return manifest
        except Exception:
            if staging.exists():
                shutil.rmtree(staging)
            raise

    def _inject(self, source: Path, output: Path, guest_bundle: Path) -> None:
        command = [
            str(
                self.inputs.repo
                / "deploy/virtual-container/guest/inject-image.sh"
            ),
            "--source-image",
            str(source),
            "--output-image",
            str(output),
            "--bundle",
            str(guest_bundle),
            "--egress-mode",
            self.inputs.egress_mode,
            "--with-viewer",
            "--socket-gid",
            str(self.inputs.socket_gid),
        ]
        if self.inputs.otel_endpoint is not None:
            command.extend(["--otel-endpoint", self.inputs.otel_endpoint])
        if self.inputs.backend == "cloud-hypervisor":
            command.extend(["--grow-mib", "128"])
        self.executor.run(command)

    def _make_config(
        self,
        source: Path,
        kernel: Path,
        staging_image: Path,
        final_image: Path,
        output: Path,
        *,
        data: bool,
    ) -> None:
        command = [
            "python3",
            str(
                self.inputs.repo
                / "deploy/virtual-container/host/prepare-stratovirt-config.py"
            ),
            "--kata-prefix",
            str(self.inputs.kata_prefix),
            "--backend",
            self.inputs.backend,
            "--base-config",
            str(source),
            "--output",
            str(output),
            "--hypervisor",
            str(self.inputs.hypervisor),
            "--kernel",
            str(kernel),
            "--image",
            str(staging_image),
            "--image-config-path",
            str(final_image),
            "--virtiofsd",
            str(self.inputs.virtiofsd),
        ]
        if data:
            command.extend(
                ["--default-vcpus", str(self.inputs.data_vcpus), "--debug"]
            )
        self.executor.run(command)

    def _manifest_document(
        self,
        cache_key: str,
        input_document: dict[str, Any],
        staging: Path,
    ) -> dict[str, Any]:
        guest_bundle = staging / "guest-bundle"
        workload_bundle = staging / "workload-bundle"
        document: dict[str, Any] = {
            "format": 2,
            "cache_key": cache_key,
            "source_commit": _source_commit(self.inputs.repo),
            "backend": self.inputs.backend,
            "runtime": self.inputs.runtime,
            "egress_mode": self.inputs.egress_mode,
            "otel_export_enabled": self.inputs.otel_endpoint is not None,
            "release": _release_hashes(self.inputs.bin_dir),
            "guest_bundle": {
                "path": "guest-bundle",
                "manifest_sha256": sha256_file(
                    guest_bundle / "MANIFEST.sha256"
                ),
            },
            "workload_bundle": {
                "path": "workload-bundle",
                "manifest_sha256": sha256_file(
                    workload_bundle / "MANIFEST.sha256"
                ),
                "actrailctl_sha256": sha256_file(
                    workload_bundle / "bin/actrailctl"
                ),
            },
            "images": {"base": "guest-base.img", "data": "guest-data.img"},
            "runtime_configs": {
                "base": "configuration-base.toml",
                "data": "configuration-data.toml",
            },
            "integrity": {
                "configuration-base.toml": sha256_file(
                    staging / "configuration-base.toml"
                ),
                "configuration-data.toml": sha256_file(
                    staging / "configuration-data.toml"
                ),
            },
            "workload_image": {"reference": self.inputs.workload_image},
            "inputs": input_document,
        }
        if self.inputs.xiaoo is not None:
            document["xiaoo"] = {
                "path": "xiaoo",
                "sha256": sha256_file(staging / "xiaoo"),
            }
        return document

    def _validate_published(self, manifest: Path) -> None:
        DeploymentArtifacts.load(
            manifest,
            bin_dir=self.inputs.bin_dir,
            expected_backend=self.inputs.backend,
            expected_runtime=self.inputs.runtime,
            require_xiaoo=self.inputs.xiaoo is not None,
        )

    def _ensure_workload_image(self) -> None:
        command = [
            "ctr",
            "-n",
            "default",
            "images",
            "list",
            "--quiet",
            f"name=={self.inputs.workload_image}",
        ]
        listed = self.executor.run(command, capture=True)
        present = self.inputs.workload_image in {
            line.strip() for line in listed.stdout.splitlines() if line.strip()
        }
        if present and self.inputs.image_pull_policy != "always":
            print("workload_image_cache=hit", flush=True)
            return
        archive = self.inputs.workload_image_archive
        if archive is not None:
            print("workload_image_cache=import", flush=True)
            self.executor.run(
                ["ctr", "-n", "default", "images", "import", str(archive)]
            )
            return
        if self.inputs.image_pull_policy == "never":
            raise RuntimeError(
                "workload image is missing and pull policy is never: "
                f"{self.inputs.workload_image}; provide --workload-image-archive"
            )
        print("workload_image_cache=pull", flush=True)
        self.executor.run(
            [
                "ctr",
                "-n",
                "default",
                "images",
                "pull",
                self.inputs.workload_image,
            ]
        )


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
            relative = _display_path(path, inputs.repo)
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


def write_test_profile(
    inputs: PreparationInputs,
    manifest: Path,
    profile_path: Path,
) -> None:
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
    if inputs.workload_image_archive is not None:
        environment["VIRTUAL_CONTAINER_E2E_IMAGE_ARCHIVE"] = str(
            inputs.workload_image_archive
        )
    path_prepend = [
        "${REPO}/local/kata/bin",
        str(inputs.hypervisor.parent),
        str(inputs.virtiofsd.parent),
        *_invoking_user_bin_dirs(),
        "/opt/kata/bin",
        "/usr/local/bin",
        "/usr/local/sbin",
        "/usr/sbin",
        "/usr/bin",
        "/bin",
    ]
    profile = {
        "format": 2,
        "name": (
            "openEuler / Kata 3.32 / "
            + BACKEND_DISPLAY_NAMES[inputs.backend]
        ),
        "path_prepend": list(dict.fromkeys(path_prepend)),
        "environment": environment,
    }
    profile_path.parent.mkdir(parents=True, exist_ok=True)
    _atomic_json(profile_path, profile)
    profile_path.chmod(0o644)


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
        repo
        / "tests/v2/regression/virtual_container/prepare-guest-bundle.sh",
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


def _release_hashes(bin_dir: Path) -> dict[str, str]:
    return {
        digest_name: sha256_file(bin_dir / filename)
        for digest_name, filename in RELEASE_FILES.items()
    }


def _source_commit(repo: Path) -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), "rev-parse", "HEAD"],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return result.stdout.strip() if result.returncode == 0 else "unknown"


def _invoking_user_bin_dirs() -> list[str]:
    user = os.environ.get("SUDO_USER")
    if not user or user == "root":
        return []
    try:
        home = Path(pwd.getpwnam(user).pw_dir)
    except KeyError:
        return []
    return [str(home / ".local/bin"), str(home / ".cargo/bin")]


def _restore_invoking_user_ownership(
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
    _chown_tree(output_root, uid, gid, recursive=False)
    _chown_tree(manifest.parent, uid, gid, recursive=True)
    if profile_path is not None:
        _chown_tree(profile_path, uid, gid, recursive=False)


def _chown_tree(path: Path, uid: int, gid: int, *, recursive: bool) -> None:
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


def _display_path(path: Path, repo: Path) -> str:
    try:
        return path.resolve().relative_to(repo.resolve()).as_posix()
    except ValueError:
        return str(path.resolve())


def _atomic_json(path: Path, document: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.",
        dir=path.parent,
        text=True,
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as destination:
            json.dump(document, destination, indent=2, sort_keys=True)
            destination.write("\n")
            destination.flush()
            os.fsync(destination.fileno())
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def _check_profile(profile_path: Path, manifest: Path) -> None:
    try:
        document = json.loads(profile_path.read_text(encoding="utf-8"))
        configured = document["environment"][
            "VIRTUAL_CONTAINER_E2E_ARTIFACT_MANIFEST"
        ]
    except (OSError, KeyError, TypeError, json.JSONDecodeError) as error:
        raise RuntimeError(f"cannot validate V2 test profile {profile_path}") from error
    if configured != str(manifest):
        raise RuntimeError(
            f"V2 test profile points at {configured}, expected {manifest}"
        )


def _fsync_tree(directory: Path) -> None:
    for path in directory.rglob("*"):
        if path.is_file():
            with path.open("rb") as source:
                os.fsync(source.fileno())
    descriptor = os.open(directory, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def _shell_display(value: str) -> str:
    if re.fullmatch(r"[A-Za-z0-9_./:=+,-]+", value):
        return value
    return json.dumps(value)
