"""Content-addressed deployment preparation for Kata V2 acceptance tests."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path, PurePosixPath
from typing import Any, Mapping, Sequence


REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT))

from tests.v2.common.kata_runtime import (  # noqa: E402
    DeploymentArtifacts,
    kata_backend,
    sha256_file,
)
from tests.v2.common.kata_runtime.image import (  # noqa: E402
    containerd_image_check_ready,
    verified_image_archive_snapshot,
)
from v2_artifacts_support import (  # noqa: E402
    PreparationInputs,
    PreparedWorkloadImage,
    V2TestProfile,
    atomic_json,
    build_input_document,
    cache_key_for,
    default_tool_inputs,
    fsync_tree,
    infer_runtime_path,
    prepare_firecracker_workload_image,
    release_hashes,
    restore_invoking_user_ownership,
    shell_display,
    source_commit,
)


class CommandExecutor:
    def run(
        self,
        command: Sequence[str],
        *,
        environment: Mapping[str, str] | None = None,
        capture: bool = False,
    ) -> subprocess.CompletedProcess[str]:
        printable = " ".join(shell_display(value) for value in command)
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
            if capture:
                diagnostic = (
                    (result.stderr or "") + (result.stdout or "")
                ).strip() or "no diagnostic output"
            else:
                diagnostic = "child output was streamed above"
            raise RuntimeError(
                f"command failed exit={result.returncode}: {printable}: "
                f"{diagnostic}"
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
            (
                workload_image,
                workload_image_archive,
                workload_image_archive_sha256,
            ) = self._workload_image_settings(manifest)
            if ensure_workload_image:
                self._ensure_workload_image(
                    reference=workload_image,
                    archive=workload_image_archive,
                    archive_sha256=workload_image_archive_sha256,
                )
            if profile_path is not None:
                if check_only:
                    V2TestProfile.validate(profile_path, manifest)
                else:
                    V2TestProfile.write(
                        self.inputs,
                        manifest,
                        profile_path,
                        workload_image=workload_image,
                        workload_image_archive=workload_image_archive,
                    )
        finally:
            restore_invoking_user_ownership(
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
            host_bundle = staging / "host-bundle"
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
            self._prepare_host_bundle(host_bundle)
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

            prepared_workload_image = self._prepare_workload_image(
                staging,
                cache_key,
                input_document,
            )

            manifest_document = self._manifest_document(
                cache_key,
                input_document,
                staging,
                prepared_workload_image,
            )
            atomic_json(staging / "manifest.json", manifest_document)
            fsync_tree(staging)
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

    def _prepare_host_bundle(self, output: Path) -> None:
        output.mkdir()
        source = self.inputs.bin_dir / "actrail-vsock-gateway"
        destination = output / source.name
        shutil.copy2(source, destination)
        destination.chmod(0o755)
        plugin_source = (
            self.inputs.repo
            / "examples/plugins/builtin/sandbox-resource-alert"
        )
        plugin_output = output / "sandbox-resource-alert"
        plugin_output.mkdir()
        for name in (
            "sandbox-resource-alert.plugin.toml",
            "sandbox-resource-alert.config.json",
            "sandbox-resource-alert.config.v1.schema.json",
        ):
            shutil.copy2(plugin_source / name, plugin_output / name)
        manifest = output / "MANIFEST.sha256"
        lines = []
        for path in sorted(output.rglob("*")):
            if not path.is_file() or path == manifest:
                continue
            relative = path.relative_to(output).as_posix()
            lines.append(f"{sha256_file(path)}  ./{relative}\n")
        manifest.write_text("".join(lines), encoding="utf-8")
        manifest.chmod(0o644)

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
            "--grow-mib",
            "128",
        ]
        if self.inputs.otel_endpoint is not None:
            command.extend(["--otel-endpoint", self.inputs.otel_endpoint])
        if self.inputs.sandbox_observer:
            command.append("--with-sandbox-observer")
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
        ]
        if self.inputs.virtiofsd is not None:
            command.extend(["--virtiofsd", str(self.inputs.virtiofsd)])
        if self.inputs.jailer is not None:
            command.extend(["--jailer", str(self.inputs.jailer)])
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
        prepared_workload_image: PreparedWorkloadImage | None,
    ) -> dict[str, Any]:
        guest_bundle = staging / "guest-bundle"
        host_bundle = staging / "host-bundle"
        workload_bundle = staging / "workload-bundle"
        document: dict[str, Any] = {
            "format": 2,
            "cache_key": cache_key,
            "source_commit": source_commit(self.inputs.repo),
            "backend": self.inputs.backend,
            "runtime": self.inputs.runtime,
            "egress_mode": self.inputs.egress_mode,
            "otel_export_enabled": self.inputs.otel_endpoint is not None,
            "sandbox_observer_enabled": self.inputs.sandbox_observer,
            "release": release_hashes(self.inputs.bin_dir),
            "guest_bundle": {
                "path": "guest-bundle",
                "manifest_sha256": sha256_file(
                    guest_bundle / "MANIFEST.sha256"
                ),
            },
            "host_bundle": {
                "path": "host-bundle",
                "manifest_sha256": sha256_file(
                    host_bundle / "MANIFEST.sha256"
                ),
                "gateway_sha256": sha256_file(
                    host_bundle / "actrail-vsock-gateway"
                ),
                "sandbox_resource_alert_manifest_sha256": sha256_file(
                    host_bundle
                    / "sandbox-resource-alert/sandbox-resource-alert.plugin.toml"
                ),
                "sandbox_resource_alert_config_sha256": sha256_file(
                    host_bundle
                    / "sandbox-resource-alert/sandbox-resource-alert.config.json"
                ),
                "sandbox_resource_alert_schema_sha256": sha256_file(
                    host_bundle
                    / "sandbox-resource-alert"
                    / "sandbox-resource-alert.config.v1.schema.json"
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
            "workload_image": self._workload_image_document(
                prepared_workload_image
            ),
            "inputs": input_document,
        }
        if self.inputs.xiaoo is not None:
            document["xiaoo"] = {
                "path": "xiaoo",
                "sha256": sha256_file(staging / "xiaoo"),
            }
        return document

    def _prepare_workload_image(
        self,
        staging: Path,
        cache_key: str,
        input_document: dict[str, Any],
    ) -> PreparedWorkloadImage | None:
        if (
            self.inputs.backend != "firecracker"
            or self.inputs.xiaoo is None
            or self.inputs.workload_image_archive is None
        ):
            return None
        files = input_document.get("files")
        if not isinstance(files, dict):
            raise RuntimeError("artifact input file digests are missing")
        archive_sha256 = files.get("workload_image_archive")
        xiaoo_sha256 = files.get("xiaoo")
        if not isinstance(archive_sha256, str) or not isinstance(
            xiaoo_sha256,
            str,
        ):
            raise RuntimeError(
                "Firecracker workload image input digests are missing"
            )
        machine = os.uname().machine.lower()
        expected_architecture = {
            "aarch64": "arm64",
            "x86_64": "amd64",
        }.get(machine, machine)
        prepared = prepare_firecracker_workload_image(
            self.inputs.workload_image_archive,
            staging / "xiaoo",
            staging / "workload-image-firecracker.docker.tar",
            cache_key,
            source_reference=self.inputs.workload_image,
            source_archive_sha256=archive_sha256,
            xiaoo_sha256=xiaoo_sha256,
            expected_architecture=expected_architecture,
        )
        if prepared.xiaoo_sha256 != xiaoo_sha256:
            raise RuntimeError(
                "prepared Firecracker workload image xiaoO digest does not "
                "match the artifact inputs"
            )
        return prepared

    def _workload_image_document(
        self,
        prepared: PreparedWorkloadImage | None,
    ) -> dict[str, Any]:
        if prepared is None:
            return {"reference": self.inputs.workload_image}
        return {
            "reference": prepared.reference,
            "archive": prepared.archive.name,
            "archive_sha256": prepared.archive_sha256,
            "preinstalled_xiaoo_path": prepared.xiaoo_path,
            "preinstalled_xiaoo_sha256": prepared.xiaoo_sha256,
        }

    def _validate_published(self, manifest: Path) -> None:
        DeploymentArtifacts.load(
            manifest,
            bin_dir=self.inputs.bin_dir,
            expected_backend=self.inputs.backend,
            expected_runtime=self.inputs.runtime,
            require_xiaoo=self.inputs.xiaoo is not None,
            require_preinstalled_xiaoo=(
                self.inputs.backend == "firecracker"
                and self.inputs.xiaoo is not None
            ),
        )

    def _workload_image_settings(
        self,
        manifest: Path,
    ) -> tuple[str, Path | None, str | None]:
        try:
            document = json.loads(manifest.read_text(encoding="utf-8"))
            workload = document["workload_image"]
            reference = workload["reference"]
        except (OSError, KeyError, TypeError, json.JSONDecodeError) as error:
            raise RuntimeError(
                f"cannot resolve workload image from artifact manifest {manifest}"
            ) from error
        if not isinstance(reference, str) or not reference:
            raise RuntimeError("artifact workload image reference is invalid")
        archive_name = workload.get("archive")
        if archive_name is None:
            return (
                reference,
                self.inputs.workload_image_archive,
                None,
            )
        if not isinstance(archive_name, str):
            raise RuntimeError("artifact workload image archive path is invalid")
        relative = PurePosixPath(archive_name)
        if relative.is_absolute() or ".." in relative.parts:
            raise RuntimeError("artifact workload image archive path is unsafe")
        archive = manifest.parent / Path(*relative.parts)
        expected_sha256 = workload.get("archive_sha256")
        if not archive.is_file() or not isinstance(expected_sha256, str):
            raise RuntimeError("artifact workload image archive is invalid")
        actual_sha256 = sha256_file(archive)
        if actual_sha256 != expected_sha256:
            raise RuntimeError(
                "artifact workload image archive digest mismatch: "
                f"expected={expected_sha256} actual={actual_sha256}"
            )
        return reference, archive, expected_sha256

    def _ensure_workload_image(
        self,
        *,
        reference: str | None = None,
        archive: Path | None = None,
        archive_sha256: str | None = None,
    ) -> None:
        reference = self.inputs.workload_image if reference is None else reference
        if archive is None:
            archive = self.inputs.workload_image_archive
        snapshotter = kata_backend(self.inputs.backend).default_snapshotter
        snapshotter_option = (
            [] if snapshotter is None else ["--snapshotter", snapshotter]
        )
        present = self._workload_image_present(snapshotter, reference)
        if present and self.inputs.image_pull_policy != "always":
            print("workload_image_cache=hit", flush=True)
            return
        if archive is not None:
            print("workload_image_cache=import", flush=True)
            if archive_sha256 is None:
                self._import_workload_image_archive(
                    archive,
                    snapshotter_option,
                )
            else:
                with verified_image_archive_snapshot(
                    archive,
                    archive_sha256,
                ) as snapshot:
                    self._import_workload_image_archive(
                        snapshot,
                        snapshotter_option,
                    )
            self._verify_snapshotter_image_ready(
                snapshotter,
                "import",
                reference,
            )
            return
        if self.inputs.image_pull_policy == "never":
            state = "missing"
            if snapshotter is not None:
                state = (
                    "missing, incomplete, or not unpacked for snapshotter "
                    + snapshotter
                )
            raise RuntimeError(
                f"workload image is {state} and pull policy is never: "
                f"{reference}; provide --workload-image-archive"
            )
        print("workload_image_cache=pull", flush=True)
        self.executor.run(
            [
                "ctr",
                "-n",
                "default",
                "images",
                "pull",
                *snapshotter_option,
                reference,
            ]
        )
        self._verify_snapshotter_image_ready(snapshotter, "pull", reference)

    def _import_workload_image_archive(
        self,
        archive: Path,
        snapshotter_option: list[str],
    ) -> None:
        self.executor.run(
            [
                "ctr",
                "-n",
                "default",
                "images",
                "import",
                *snapshotter_option,
                str(archive),
            ]
        )

    def _workload_image_present(
        self,
        snapshotter: str | None,
        reference: str | None = None,
    ) -> bool:
        reference = self.inputs.workload_image if reference is None else reference
        if snapshotter is None:
            command = [
                "ctr",
                "-n",
                "default",
                "images",
                "list",
                "--quiet",
                f"name=={reference}",
            ]
        else:
            command = [
                "ctr",
                "-n",
                "default",
                "images",
                "check",
                "--snapshotter",
                snapshotter,
                f"name=={reference}",
            ]
        listed = self.executor.run(command, capture=True)
        if snapshotter is None:
            return reference in {
                line.strip()
                for line in listed.stdout.splitlines()
                if line.strip()
            }
        return containerd_image_check_ready(
            listed.stdout,
            reference,
        )

    def _verify_snapshotter_image_ready(
        self,
        snapshotter: str | None,
        operation: str,
        reference: str | None = None,
    ) -> None:
        if snapshotter is None:
            return
        reference = self.inputs.workload_image if reference is None else reference
        if not self._workload_image_present(snapshotter, reference):
            raise RuntimeError(
                f"workload image {operation} completed but {reference} "
                "is incomplete or not unpacked "
                f"for snapshotter {snapshotter}"
            )
