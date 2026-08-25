from __future__ import annotations

import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from tests.v2.common.kata_runtime.artifacts import (
    DeploymentArtifacts,
    DirectoryManifestRequirement,
    validate_release_bundle_consistency,
)
from tests.v2.common.kata_runtime.requirements import PreparePolicy
from tests.v2.common.kata_runtime.runtime_config import (
    REQUIRED_EBPF_KERNEL_CONFIG,
)


class DirectoryManifestRequirementTest(unittest.TestCase):
    def test_cache_hit_validates_manifest_without_running_prepare_command(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-artifact.") as raw_dir:
            bundle = Path(raw_dir) / "bundle"
            bundle.mkdir()
            payload = bundle / "bin"
            payload.write_bytes(b"actrail")
            digest = hashlib.sha256(b"actrail").hexdigest()
            manifest_content = f"{digest}  ./bin\n"
            (bundle / "MANIFEST.sha256").write_text(
                manifest_content,
                encoding="utf-8",
            )
            requirement = DirectoryManifestRequirement(bundle)

            requirement.ensure(PreparePolicy.CHECK_ONLY)

        self.assertEqual(
            requirement.digest,
            hashlib.sha256(manifest_content.encode()).hexdigest(),
        )


class DeploymentArtifactsTest(unittest.TestCase):
    def test_execution_isolation_requires_a_guest_system_observer(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(Path(raw_dir))

            with self.assertRaisesRegex(
                RuntimeError,
                "Guest system sandbox observer",
            ):
                DeploymentArtifacts.load(
                    manifest,
                    bin_dir=bin_dir,
                    expected_backend="stratovirt",
                    expected_runtime="io.containerd.kata332.v2",
                    require_sandbox_observer=True,
                )

    def test_loads_matching_content_addressed_artifact(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(Path(raw_dir), with_xiaoo=True)

            resolved = DeploymentArtifacts.load(
                manifest,
                bin_dir=bin_dir,
                expected_backend="stratovirt",
                expected_runtime="io.containerd.kata332.v2",
                require_xiaoo=True,
            )

        self.assertEqual(resolved.manifest, manifest.resolve())
        self.assertEqual(resolved.backend, "stratovirt")
        self.assertEqual(resolved.workload_image, "example.test/workload:24.09")
        self.assertEqual(resolved.xiaoo, (manifest.parent / "xiaoo").resolve())

    def test_loads_matching_cloud_hypervisor_artifact(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(
                Path(raw_dir),
                backend="cloud-hypervisor",
            )

            resolved = DeploymentArtifacts.load(
                manifest,
                bin_dir=bin_dir,
                expected_backend="cloud-hypervisor",
                expected_runtime="io.containerd.kata332.v2",
            )
            base_config = resolved.base_config.read_text(encoding="utf-8")

        self.assertEqual(resolved.backend, "cloud-hypervisor")
        self.assertIn("[hypervisor.clh]", base_config)

    def test_loads_firecracker_artifact_without_virtiofs_input(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(
                Path(raw_dir),
                backend="firecracker",
            )

            resolved = DeploymentArtifacts.load(
                manifest,
                bin_dir=bin_dir,
                expected_backend="firecracker",
                expected_runtime="io.containerd.kata332.v2",
                require_sandbox_observer=True,
            )
            document = json.loads(manifest.read_text(encoding="utf-8"))
            base_config = resolved.base_config.read_text(encoding="utf-8")

        self.assertEqual(resolved.backend, "firecracker")
        self.assertTrue(resolved.sandbox_observer_enabled)
        self.assertIn("[hypervisor.firecracker]", base_config)
        self.assertNotIn("virtio_fs_daemon", base_config)
        self.assertNotIn("virtiofsd", document["inputs"]["files"])
        self.assertNotIn("virtiofsd", document["inputs"]["paths"])
        self.assertIn("jailer", document["inputs"]["files"])
        self.assertIn("jailer_path", base_config)

    def test_firecracker_execution_requires_preinstalled_xiaoo_image(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(
                Path(raw_dir),
                backend="firecracker",
                with_xiaoo=True,
            )

            with self.assertRaisesRegex(RuntimeError, "preinstalled xiaoO"):
                DeploymentArtifacts.load(
                    manifest,
                    bin_dir=bin_dir,
                    expected_backend="firecracker",
                    expected_runtime="io.containerd.kata332.v2",
                    require_xiaoo=True,
                    require_preinstalled_xiaoo=True,
                )

    def test_firecracker_preinstalled_image_reference_is_bound_to_cache_key(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(
                Path(raw_dir),
                backend="firecracker",
                with_xiaoo=True,
            )
            document = json.loads(manifest.read_text(encoding="utf-8"))
            archive = manifest.parent / "workload-image.docker.tar"
            archive.write_bytes(b"derived workload archive")
            xiaoo_sha256 = document["xiaoo"]["sha256"]
            document["workload_image"] = {
                "reference": "example.test/tampered:latest",
                "archive": archive.name,
                "archive_sha256": _sha256(archive),
                "preinstalled_xiaoo_path": (
                    "/opt/actrail-execution/xiaoo-real"
                ),
                "preinstalled_xiaoo_sha256": xiaoo_sha256,
            }
            document["inputs"]["files"]["workload_image_archive"] = "b" * 64
            manifest.write_text(json.dumps(document), encoding="utf-8")

            with self.assertRaisesRegex(RuntimeError, "cache_key"):
                DeploymentArtifacts.load(
                    manifest,
                    bin_dir=bin_dir,
                    expected_backend="firecracker",
                    expected_runtime="io.containerd.kata332.v2",
                    require_xiaoo=True,
                    require_preinstalled_xiaoo=True,
                )

    def test_observer_rejects_missing_data_kernel_config_input(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(
                Path(raw_dir),
                backend="firecracker",
            )
            document = json.loads(manifest.read_text(encoding="utf-8"))
            document["inputs"]["files"].pop("data_kernel_config")
            document["inputs"]["paths"].pop("data_kernel_config")
            manifest.write_text(json.dumps(document), encoding="utf-8")

            with self.assertRaisesRegex(RuntimeError, "data_kernel_config"):
                DeploymentArtifacts.load(
                    manifest,
                    bin_dir=bin_dir,
                    expected_backend="firecracker",
                    expected_runtime="io.containerd.kata332.v2",
                    require_sandbox_observer=True,
                )

    def test_observer_rejects_changed_data_kernel_config(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(
                Path(raw_dir),
                backend="firecracker",
            )
            document = json.loads(manifest.read_text(encoding="utf-8"))
            kernel_config = Path(
                document["inputs"]["paths"]["data_kernel_config"]
            )
            kernel_config.write_text("CONFIG_BPF=y\n", encoding="utf-8")

            with self.assertRaisesRegex(
                RuntimeError,
                "data_kernel_config checksum mismatch",
            ):
                DeploymentArtifacts.load(
                    manifest,
                    bin_dir=bin_dir,
                    expected_backend="firecracker",
                    expected_runtime="io.containerd.kata332.v2",
                    require_sandbox_observer=True,
                )

    def test_observer_rejects_unrelated_data_kernel_config(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(
                Path(raw_dir),
                backend="firecracker",
            )
            document = json.loads(manifest.read_text(encoding="utf-8"))
            foreign = Path(raw_dir) / "foreign-kernel.config"
            foreign.write_text(
                "\n".join(REQUIRED_EBPF_KERNEL_CONFIG) + "\n",
                encoding="utf-8",
            )
            document["inputs"]["files"]["data_kernel_config"] = _sha256(
                foreign
            )
            document["inputs"]["paths"]["data_kernel_config"] = str(foreign)
            manifest.write_text(json.dumps(document), encoding="utf-8")

            with self.assertRaisesRegex(
                RuntimeError,
                "does not match the data kernel",
            ):
                DeploymentArtifacts.load(
                    manifest,
                    bin_dir=bin_dir,
                    expected_backend="firecracker",
                    expected_runtime="io.containerd.kata332.v2",
                    require_sandbox_observer=True,
                )

    def test_observer_rejects_missing_data_kernel_capability(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(
                Path(raw_dir),
                backend="firecracker",
            )
            document = json.loads(manifest.read_text(encoding="utf-8"))
            kernel_config = Path(
                document["inputs"]["paths"]["data_kernel_config"]
            )
            kernel_config.write_text("CONFIG_BPF=y\n", encoding="utf-8")
            document["inputs"]["files"]["data_kernel_config"] = _sha256(
                kernel_config
            )
            manifest.write_text(json.dumps(document), encoding="utf-8")

            with self.assertRaisesRegex(
                RuntimeError,
                "CONFIG_DEBUG_INFO_BTF=y",
            ):
                DeploymentArtifacts.load(
                    manifest,
                    bin_dir=bin_dir,
                    expected_backend="firecracker",
                    expected_runtime="io.containerd.kata332.v2",
                    require_sandbox_observer=True,
                )

    def test_firecracker_rejects_jailer_changed_after_prepare(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(
                Path(raw_dir),
                backend="firecracker",
            )
            document = json.loads(manifest.read_text(encoding="utf-8"))
            jailer = Path(document["inputs"]["paths"]["jailer"])
            jailer.write_bytes(b"replaced jailer")
            jailer.chmod(0o755)

            with self.assertRaisesRegex(
                RuntimeError,
                "runtime input jailer checksum mismatch",
            ):
                DeploymentArtifacts.load(
                    manifest,
                    bin_dir=bin_dir,
                    expected_backend="firecracker",
                    expected_runtime="io.containerd.kata332.v2",
                    require_sandbox_observer=True,
                )

    def test_firecracker_rejects_nonexecutable_manifest_jailer(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(
                Path(raw_dir),
                backend="firecracker",
            )
            document = json.loads(manifest.read_text(encoding="utf-8"))
            Path(document["inputs"]["paths"]["jailer"]).chmod(0o644)

            with self.assertRaisesRegex(
                RuntimeError,
                "Firecracker jailer is not executable",
            ):
                DeploymentArtifacts.load(
                    manifest,
                    bin_dir=bin_dir,
                    expected_backend="firecracker",
                    expected_runtime="io.containerd.kata332.v2",
                    require_sandbox_observer=True,
                )

    def test_firecracker_rejects_a_broader_jailer_allowlist(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(
                Path(raw_dir),
                backend="firecracker",
            )
            config = manifest.parent / "configuration-base.toml"
            content = config.read_text(encoding="utf-8")
            config.write_text(
                content.replace(
                    "valid_jailer_paths = [",
                    'valid_jailer_paths = ["/opt/kata/bin/jailer", ',
                ),
                encoding="utf-8",
            )
            document = json.loads(manifest.read_text(encoding="utf-8"))
            document["integrity"]["configuration-base.toml"] = _sha256(config)
            manifest.write_text(json.dumps(document), encoding="utf-8")

            with self.assertRaisesRegex(
                RuntimeError,
                "does not restrict valid_jailer_paths",
            ):
                DeploymentArtifacts.load(
                    manifest,
                    bin_dir=bin_dir,
                    expected_backend="firecracker",
                    expected_runtime="io.containerd.kata332.v2",
                    require_sandbox_observer=True,
                )

    def test_rejects_release_binary_that_changed_after_prepare(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(Path(raw_dir))
            (bin_dir / "actrailctl").write_bytes(b"new release")

            with self.assertRaisesRegex(RuntimeError, "release.*actrailctl"):
                DeploymentArtifacts.load(
                    manifest,
                    bin_dir=bin_dir,
                    expected_backend="stratovirt",
                    expected_runtime="io.containerd.kata332.v2",
                )

    def test_rejects_runtime_config_pointing_at_another_image(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(Path(raw_dir))
            config = manifest.parent / "configuration-base.toml"
            content = config.read_text(encoding="utf-8")
            config.write_text(
                content.replace(
                    f'image = "{manifest.parent / "guest-base.img"}"',
                    'image = "/tmp/foreign.img"',
                ),
                encoding="utf-8",
            )
            document = json.loads(manifest.read_text(encoding="utf-8"))
            document["integrity"]["configuration-base.toml"] = _sha256(config)
            manifest.write_text(json.dumps(document), encoding="utf-8")

            with self.assertRaisesRegex(RuntimeError, "does not reference"):
                DeploymentArtifacts.load(
                    manifest,
                    bin_dir=bin_dir,
                    expected_backend="stratovirt",
                    expected_runtime="io.containerd.kata332.v2",
                )

    def test_rejects_runtime_assets_that_changed_after_prepare(self) -> None:
        cases = (
            ("base-kernel", r"runtime input base_kernel.*checksum mismatch"),
            ("data-kernel", r"runtime input data_kernel.*checksum mismatch"),
            ("stratovirt", r"runtime input hypervisor.*checksum mismatch"),
            ("virtiofsd", r"runtime input virtiofsd.*checksum mismatch"),
        )
        for name, diagnostic in cases:
            with self.subTest(asset=name), tempfile.TemporaryDirectory(
                prefix="actrail-deployment."
            ) as raw_dir:
                temporary = Path(raw_dir)
                manifest, bin_dir = _deployment_fixture(temporary)
                (temporary / "runtime" / name).write_bytes(b"stale runtime asset")

                with self.assertRaisesRegex(RuntimeError, diagnostic):
                    DeploymentArtifacts.load(
                        manifest,
                        bin_dir=bin_dir,
                        expected_backend="stratovirt",
                        expected_runtime="io.containerd.kata332.v2",
                    )

    def test_rejects_runtime_config_pointing_at_unrecorded_vmm(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(Path(raw_dir))
            config = manifest.parent / "configuration-base.toml"
            config.write_text(
                config.read_text(encoding="utf-8").replace(
                    'path = "',
                    'path = "/tmp/unrecorded-',
                    1,
                ),
                encoding="utf-8",
            )
            document = json.loads(manifest.read_text(encoding="utf-8"))
            document["integrity"]["configuration-base.toml"] = _sha256(config)
            manifest.write_text(json.dumps(document), encoding="utf-8")

            with self.assertRaisesRegex(RuntimeError, "manifest input path"):
                DeploymentArtifacts.load(
                    manifest,
                    bin_dir=bin_dir,
                    expected_backend="stratovirt",
                    expected_runtime="io.containerd.kata332.v2",
                )

    def test_rejects_artifact_path_that_escapes_digest_directory(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(Path(raw_dir))
            document = json.loads(manifest.read_text(encoding="utf-8"))
            document["images"]["base"] = "../foreign.img"
            manifest.write_text(json.dumps(document), encoding="utf-8")

            with self.assertRaisesRegex(RuntimeError, "unsafe artifact path"):
                DeploymentArtifacts.load(
                    manifest,
                    bin_dir=bin_dir,
                    expected_backend="stratovirt",
                    expected_runtime="io.containerd.kata332.v2",
                )

    def test_xiaoo_is_only_required_by_concurrency_case(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(Path(raw_dir))

            with self.assertRaisesRegex(RuntimeError, "xiaoO"):
                DeploymentArtifacts.load(
                    manifest,
                    bin_dir=bin_dir,
                    expected_backend="stratovirt",
                    expected_runtime="io.containerd.kata332.v2",
                    require_xiaoo=True,
                )

    def test_legacy_bundles_must_match_current_release(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-deployment.") as raw_dir:
            manifest, bin_dir = _deployment_fixture(Path(raw_dir))
            root = manifest.parent
            validate_release_bundle_consistency(
                bin_dir,
                root / "guest-bundle",
                root / "workload-bundle",
            )
            (root / "workload-bundle/bin/actrailctl").write_bytes(b"stale")
            _write_directory_manifest(root / "workload-bundle")

            with self.assertRaisesRegex(RuntimeError, "workload.*actrailctl"):
                validate_release_bundle_consistency(
                    bin_dir,
                    root / "guest-bundle",
                    root / "workload-bundle",
                )


_RELEASE_FILES = (
    "actraild",
    "actrailctl",
    "actrail-sb",
    "actrail-vsock-gateway",
    "actrailviewer",
    "libactrail_tls_payload_probe_sync.so",
)


def _deployment_fixture(
    temporary: Path,
    *,
    with_xiaoo: bool = False,
    backend: str = "stratovirt",
) -> tuple[Path, Path]:
    cache_key = "a" * 64
    root = temporary / cache_key
    root.mkdir()
    bin_dir = temporary / "release"
    bin_dir.mkdir()
    guest = root / "guest-bundle"
    host = root / "host-bundle"
    workload = root / "workload-bundle"
    (workload / "bin").mkdir(parents=True)
    guest.mkdir()
    host.mkdir()
    host_plugin = host / "sandbox-resource-alert"
    host_plugin.mkdir()
    for name in (
        "sandbox-resource-alert.plugin.toml",
        "sandbox-resource-alert.config.json",
        "sandbox-resource-alert.config.v1.schema.json",
    ):
        (host_plugin / name).write_text(f"fixture:{name}\n", encoding="utf-8")

    release = {}
    release_key = {
        "actraild": "actraild_sha256",
        "actrailctl": "actrailctl_sha256",
        "actrail-sb": "actrail_sb_sha256",
        "actrail-vsock-gateway": "actrail_vsock_gateway_sha256",
        "actrailviewer": "actrailviewer_sha256",
        "libactrail_tls_payload_probe_sync.so": "tls_probe_sha256",
    }
    for name in _RELEASE_FILES:
        content = f"release:{name}".encode()
        (bin_dir / name).write_bytes(content)
        if name == "actrail-vsock-gateway":
            (host / name).write_bytes(content)
            (host / name).chmod(0o755)
        else:
            (guest / name).write_bytes(content)
            if name == "actrail-sb":
                (guest / name).chmod(0o755)
        release[release_key[name]] = hashlib.sha256(content).hexdigest()
    (workload / "bin/actrailctl").write_bytes(
        (bin_dir / "actrailctl").read_bytes()
    )
    _write_directory_manifest(guest)
    _write_directory_manifest(host)
    _write_directory_manifest(workload)

    base_image = root / "guest-base.img"
    data_image = root / "guest-data.img"
    base_image.write_bytes(b"base image")
    data_image.write_bytes(b"data image")
    base_config = root / "configuration-base.toml"
    data_config = root / "configuration-data.toml"
    runtime = temporary / "runtime"
    runtime.mkdir()
    runtime_files = {}
    hypervisor_name = {
        "stratovirt": "stratovirt",
        "cloud-hypervisor": "cloud-hypervisor",
        "firecracker": "firecracker",
    }[backend]
    runtime_names = [hypervisor_name, "base-kernel", "data-kernel"]
    if backend != "firecracker":
        runtime_names.append("virtiofsd")
    else:
        runtime_names.append("jailer")
    for name in runtime_names:
        path = runtime / name
        path.write_bytes(f"runtime:{name}".encode())
        if name in {
            "stratovirt",
            "cloud-hypervisor",
            "firecracker",
            "jailer",
            "virtiofsd",
        }:
            path.chmod(0o755)
        runtime_files[name] = path
    data_kernel_config = Path(f'{runtime_files["data-kernel"]}.config')
    data_kernel_config.write_text(
        "\n".join(REQUIRED_EBPF_KERNEL_CONFIG) + "\n",
        encoding="utf-8",
    )
    base_config.write_text(
        _runtime_config(
            image=base_image,
            hypervisor=runtime_files[hypervisor_name],
            kernel=runtime_files["base-kernel"],
            virtiofsd=runtime_files.get("virtiofsd"),
            jailer=runtime_files.get("jailer"),
            backend=backend,
        ),
        encoding="utf-8",
    )
    data_config.write_text(
        _runtime_config(
            image=data_image,
            hypervisor=runtime_files[hypervisor_name],
            kernel=runtime_files["data-kernel"],
            virtiofsd=runtime_files.get("virtiofsd"),
            jailer=runtime_files.get("jailer"),
            backend=backend,
        ),
        encoding="utf-8",
    )

    document = {
        "format": 2,
        "cache_key": cache_key,
        "source_commit": "deadbeef",
        "backend": backend,
        "runtime": "io.containerd.kata332.v2",
        "sandbox_observer_enabled": backend == "firecracker",
        "release": release,
        "guest_bundle": {
            "path": "guest-bundle",
            "manifest_sha256": _sha256(guest / "MANIFEST.sha256"),
        },
        "host_bundle": {
            "path": "host-bundle",
            "manifest_sha256": _sha256(host / "MANIFEST.sha256"),
            "gateway_sha256": _sha256(host / "actrail-vsock-gateway"),
            "sandbox_resource_alert_manifest_sha256": _sha256(
                host_plugin / "sandbox-resource-alert.plugin.toml"
            ),
            "sandbox_resource_alert_config_sha256": _sha256(
                host_plugin / "sandbox-resource-alert.config.json"
            ),
            "sandbox_resource_alert_schema_sha256": _sha256(
                host_plugin / "sandbox-resource-alert.config.v1.schema.json"
            ),
        },
        "workload_bundle": {
            "path": "workload-bundle",
            "manifest_sha256": _sha256(workload / "MANIFEST.sha256"),
            "actrailctl_sha256": _sha256(workload / "bin/actrailctl"),
        },
        "images": {"base": "guest-base.img", "data": "guest-data.img"},
        "runtime_configs": {
            "base": "configuration-base.toml",
            "data": "configuration-data.toml",
        },
        "integrity": {
            "configuration-base.toml": _sha256(base_config),
            "configuration-data.toml": _sha256(data_config),
        },
        "workload_image": {"reference": "example.test/workload:24.09"},
        "inputs": {
            "format": 1,
            "files": {
                "hypervisor": _sha256(runtime_files[hypervisor_name]),
                "base_kernel": _sha256(runtime_files["base-kernel"]),
                "data_kernel": _sha256(runtime_files["data-kernel"]),
            },
            "paths": {
                "hypervisor": str(runtime_files[hypervisor_name]),
                "base_kernel": str(runtime_files["base-kernel"]),
                "data_kernel": str(runtime_files["data-kernel"]),
            },
        },
    }
    if "virtiofsd" in runtime_files:
        document["inputs"]["files"]["virtiofsd"] = _sha256(
            runtime_files["virtiofsd"]
        )
        document["inputs"]["paths"]["virtiofsd"] = str(
            runtime_files["virtiofsd"]
        )
    if "jailer" in runtime_files:
        document["inputs"]["files"]["jailer"] = _sha256(
            runtime_files["jailer"]
        )
        document["inputs"]["paths"]["jailer"] = str(runtime_files["jailer"])
    if document["sandbox_observer_enabled"]:
        document["inputs"]["files"]["data_kernel_config"] = _sha256(
            data_kernel_config
        )
        document["inputs"]["paths"]["data_kernel_config"] = str(
            data_kernel_config
        )
    if with_xiaoo:
        xiaoo = root / "xiaoo"
        xiaoo.write_bytes(b"xiaoo")
        xiaoo.chmod(0o755)
        document["xiaoo"] = {"path": "xiaoo", "sha256": _sha256(xiaoo)}
        document["inputs"]["files"]["xiaoo"] = _sha256(xiaoo)
        document["inputs"]["paths"]["xiaoo"] = str(xiaoo)
    manifest = root / "manifest.json"
    manifest.write_text(json.dumps(document), encoding="utf-8")
    return manifest, bin_dir


def _runtime_config(
    *,
    image: Path,
    hypervisor: Path,
    kernel: Path,
    virtiofsd: Path | None,
    jailer: Path | None,
    backend: str,
) -> str:
    section = {
        "stratovirt": "stratovirt",
        "cloud-hypervisor": "clh",
        "firecracker": "firecracker",
    }[backend]
    content = (
        f"[hypervisor.{section}]\n"
        f'path = "{hypervisor}"\n'
        f'kernel = "{kernel}"\n'
        f'image = "{image}"\n'
    )
    if virtiofsd is not None:
        content += f'virtio_fs_daemon = "{virtiofsd}"\n'
    if jailer is not None:
        content += (
            f'jailer_path = "{jailer}"\n'
            f'valid_jailer_paths = ["{jailer}"]\n'
        )
    return content


def _write_directory_manifest(directory: Path) -> None:
    lines = []
    for path in sorted(directory.rglob("*")):
        if not path.is_file() or path.name == "MANIFEST.sha256":
            continue
        relative = path.relative_to(directory).as_posix()
        lines.append(f"{_sha256(path)}  ./{relative}\n")
    (directory / "MANIFEST.sha256").write_text("".join(lines), encoding="utf-8")


def _sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


if __name__ == "__main__":
    unittest.main()
