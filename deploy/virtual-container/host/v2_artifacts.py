"""Content-addressed deployment preparation for Kata V2 acceptance tests."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any, Mapping, Sequence


REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT))

from tests.v2.common.kata_runtime import (  # noqa: E402
    DeploymentArtifacts,
    sha256_file,
)
from v2_artifacts_support import (  # noqa: E402
    PreparationInputs,
    V2TestProfile,
    atomic_json,
    build_input_document,
    cache_key_for,
    default_tool_inputs,
    fsync_tree,
    infer_runtime_path,
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
                    V2TestProfile.validate(profile_path, manifest)
                else:
                    V2TestProfile.write(self.inputs, manifest, profile_path)
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

            manifest_document = self._manifest_document(
                cache_key,
                input_document,
                staging,
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
