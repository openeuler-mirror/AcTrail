from __future__ import annotations

import json
import hashlib
import io
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from contextlib import redirect_stdout
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch


REPO = Path(__file__).resolve().parents[5]
HOST_TOOLS = REPO / "deploy/virtual-container/host"
sys.path.insert(0, str(HOST_TOOLS))

from v2_artifacts import (  # noqa: E402
    ArtifactPreparer,
    PreparationInputs,
    build_input_document,
    cache_key_for,
    default_tool_inputs,
)
from v2_artifacts_support import (  # noqa: E402
    V2TestProfile,
    prepare_firecracker_workload_image,
)
from tests.v2.common.kata_runtime import DeploymentArtifacts  # noqa: E402


class ArtifactPreparerTest(unittest.TestCase):
    def test_firecracker_reference_recipe_is_content_addressed(self) -> None:
        self.assertIn(
            REPO / "tests/v2/common/kata_runtime/image.py",
            default_tool_inputs(REPO),
        )

    def test_sandbox_observer_images_are_content_addressed_and_declared(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            disabled = _inputs(Path(raw_dir))
            enabled = replace(disabled, sandbox_observer=True)
            executor = _FakeExecutor(enabled.bin_dir)
            enabled_inputs = build_input_document(enabled)

            self.assertNotEqual(
                cache_key_for(build_input_document(disabled)),
                cache_key_for(enabled_inputs),
            )
            with redirect_stdout(io.StringIO()):
                manifest = ArtifactPreparer(enabled, executor).prepare(
                    profile_path=None,
                    ensure_workload_image=False,
                )
            document = json.loads(manifest.read_text(encoding="utf-8"))
            deployment = DeploymentArtifacts.load(
                manifest,
                bin_dir=enabled.bin_dir,
                expected_backend=enabled.backend,
                expected_runtime=enabled.runtime,
            )
            image_calls = [
                call
                for call in executor.calls
                if Path(call[0]).name == "inject-image.sh"
            ]

        self.assertTrue(document["sandbox_observer_enabled"])
        self.assertTrue(deployment.sandbox_observer_enabled)
        self.assertEqual(
            enabled_inputs["paths"]["data_kernel_config"],
            str(Path(f"{enabled.data_kernel}.config").resolve()),
        )
        self.assertEqual(len(image_calls), 2)
        for call in image_calls:
            self.assertIn("--with-sandbox-observer", call)
            self.assertEqual(call[call.index("--grow-mib") + 1], "128")

    def test_sandbox_observer_rejects_data_kernel_without_ebpf(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            inputs = replace(
                _inputs(Path(raw_dir)),
                sandbox_observer=True,
            )
            Path(f"{inputs.data_kernel}.config").write_text(
                "CONFIG_BPF=y\n",
                encoding="utf-8",
            )

            with self.assertRaisesRegex(
                ValueError,
                "sandbox observer data kernel.*CONFIG_DEBUG_INFO_BTF=y",
            ):
                inputs.validate()

    def test_sandbox_observer_rejects_data_kernel_without_config(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            inputs = replace(
                _inputs(Path(raw_dir)),
                sandbox_observer=True,
            )
            Path(f"{inputs.data_kernel}.config").unlink()

            with self.assertRaisesRegex(
                ValueError,
                "sandbox observer data kernel config is missing",
            ):
                inputs.validate()

    def test_sandbox_observer_kernel_config_changes_cache_key(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            inputs = replace(
                _inputs(Path(raw_dir)),
                sandbox_observer=True,
            )
            first = cache_key_for(build_input_document(inputs))
            kernel_config = Path(f"{inputs.data_kernel}.config")
            kernel_config.write_text(
                kernel_config.read_text(encoding="utf-8")
                + "CONFIG_LOCALVERSION=\"-changed\"\n",
                encoding="utf-8",
            )

            self.assertNotEqual(
                first,
                cache_key_for(build_input_document(inputs)),
            )

    def test_cache_key_is_stable_until_an_input_changes(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            inputs = _inputs(Path(raw_dir))
            first = cache_key_for(build_input_document(inputs))
            second = cache_key_for(build_input_document(inputs))
            self.assertEqual(first, second)

            inputs.data_image_source.write_bytes(b"changed data image")
            changed = cache_key_for(build_input_document(inputs))
            self.assertNotEqual(first, changed)

    def test_firecracker_image_inputs_each_change_the_derived_cache_key(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            base = _inputs(temporary, backend="firecracker")
            xiaoo = temporary / "xiaoo"
            xiaoo.write_bytes(b"xiaoo-v1\n")
            xiaoo.chmod(0o755)
            archive = temporary / "workload.docker.tar"
            _write_docker_archive(archive, base.workload_image)
            recipe = temporary / "workload-image-recipe.py"
            recipe.write_text("recipe-v1\n", encoding="utf-8")
            inputs = replace(
                base,
                sandbox_observer=True,
                xiaoo=xiaoo,
                workload_image_archive=archive,
                tool_inputs=(recipe,),
            )
            first = cache_key_for(build_input_document(inputs))

            xiaoo.write_bytes(b"xiaoo-v2\n")
            xiaoo_changed = cache_key_for(build_input_document(inputs))
            xiaoo.write_bytes(b"xiaoo-v1\n")

            with archive.open("ab") as destination:
                destination.write(b"archive-change")
            archive_changed = cache_key_for(build_input_document(inputs))
            _write_docker_archive(archive, base.workload_image)

            recipe.write_text("recipe-v2\n", encoding="utf-8")
            recipe_changed = cache_key_for(build_input_document(inputs))

        self.assertNotEqual(first, xiaoo_changed)
        self.assertNotEqual(first, archive_changed)
        self.assertNotEqual(first, recipe_changed)

    def test_otel_endpoint_is_optional_and_participates_in_cache_key(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            inputs = _inputs(Path(raw_dir))
            first = cache_key_for(build_input_document(inputs))
            changed_inputs = replace(
                inputs,
                otel_endpoint="https://collector.example:4318/v1/traces",
            )
            changed = cache_key_for(build_input_document(changed_inputs))

            self.assertNotEqual(first, changed)
            inputs.validate()
            with self.assertRaisesRegex(
                ValueError,
                "Guest OTLP/HTTP endpoint must be omitted or non-empty",
            ):
                replace(inputs, otel_endpoint="").validate()

    def test_vsock_egress_requires_an_otel_endpoint(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            inputs = replace(
                _inputs(Path(raw_dir)),
                egress_mode="vsock-bridge",
            )

            with self.assertRaisesRegex(
                ValueError,
                "vsock-bridge egress requires a Guest OTLP/HTTP endpoint",
            ):
                inputs.validate()

    def test_vsock_egress_mode_is_recorded_and_changes_cache_key(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            network = _inputs(Path(raw_dir))
            vsock = replace(
                network,
                egress_mode="vsock-bridge",
                otel_endpoint="http://127.0.0.1:14318/v1/traces",
            )

            network_document = build_input_document(network)
            vsock_document = build_input_document(vsock)

        self.assertEqual(vsock_document["egress_mode"], "vsock-bridge")
        self.assertNotEqual(
            cache_key_for(network_document),
            cache_key_for(vsock_document),
        )

    def test_artifact_prepare_rejects_an_unknown_egress_mode(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            inputs = replace(_inputs(Path(raw_dir)), egress_mode="host-network")

            with self.assertRaisesRegex(
                ValueError,
                "egress mode must be network or vsock-bridge",
            ):
                inputs.validate()

    def test_prepare_cli_exposes_the_guest_egress_mode(self) -> None:
        command = HOST_TOOLS / "prepare-v2-test-artifacts.py"
        completed = subprocess.run(
            [sys.executable, str(command), "--help"],
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn(
            "--backend {stratovirt,cloud-hypervisor,firecracker}",
            completed.stdout,
        )
        self.assertIn("--egress-mode {network,vsock-bridge}", completed.stdout)
        self.assertIn("--with-sandbox-observer", completed.stdout)

    def test_prepare_cli_resolves_default_firecracker_without_virtiofsd(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            inputs = _inputs(temporary, backend="firecracker")
            prefix = temporary / "kata"
            defaults = prefix / "share/defaults/kata-containers"
            defaults.mkdir(parents=True)
            (prefix / "VERSION").write_text("3.32.0\n", encoding="utf-8")
            (defaults / "configuration-fc.toml").write_text(
                "[hypervisor.firecracker]\n"
                f'path = "{inputs.hypervisor}"\n'
                f'kernel = "{inputs.base_kernel}"\n'
                f'image = "{inputs.base_image_source}"\n',
                encoding="utf-8",
            )

            completed = subprocess.run(
                [
                    sys.executable,
                    str(HOST_TOOLS / "prepare-v2-test-artifacts.py"),
                    "--backend",
                    "firecracker",
                    "--kata-prefix",
                    str(prefix),
                    "--bin-dir",
                    str(inputs.bin_dir),
                    "--output-root",
                    str(temporary / "artifacts"),
                    "--no-write-profile",
                    "--with-sandbox-observer",
                    "--skip-workload-image",
                    "--check-only",
                ],
                check=False,
                capture_output=True,
                text=True,
            )

        self.assertEqual(completed.returncode, 1)
        self.assertIn("content-addressed artifact is missing", completed.stderr)
        self.assertNotIn("virtio_fs_daemon", completed.stderr)

    def test_profile_format_two_uses_manifest_as_the_path_source(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            inputs = _inputs(temporary)
            profile = temporary / "v2-test-profile.json"
            manifest = temporary / ("a" * 64) / "manifest.json"
            manifest.parent.mkdir()
            manifest.write_text("{}", encoding="utf-8")

            V2TestProfile.write(inputs, manifest, profile)
            document = json.loads(profile.read_text(encoding="utf-8"))

        self.assertEqual(document["format"], 2)
        environment = document["environment"]
        self.assertEqual(
            environment["VIRTUAL_CONTAINER_E2E_ARTIFACT_MANIFEST"],
            str(manifest),
        )
        self.assertEqual(environment["VIRTUAL_CONTAINER_E2E_SCOPE"], "auto")
        self.assertEqual(
            environment[
                "EXECUTION_ISOLATION_STRATOVIRT_E2E_ARTIFACT_MANIFEST"
            ],
            str(manifest),
        )
        self.assertNotIn(
            "EXECUTION_ISOLATION_FIRECRACKER_E2E_ARTIFACT_MANIFEST",
            environment,
        )
        self.assertNotIn(
            "EXECUTION_ISOLATION_FIRECRACKER_E2E_ARTIFACT_SOURCE_BACKEND",
            environment,
        )
        self.assertNotIn("WORKLOAD_BUNDLE_DIR", environment)
        self.assertNotIn("VIRTUAL_CONTAINER_E2E_STRATOVIRT_CONFIG", environment)

    def test_second_prepare_reuses_published_digest_without_commands(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            inputs = _inputs(Path(raw_dir))
            executor = _FakeExecutor(inputs.bin_dir)
            preparer = ArtifactPreparer(inputs, executor)

            with redirect_stdout(io.StringIO()):
                first = preparer.prepare(
                    profile_path=None,
                    ensure_workload_image=False,
                )
                first_call_count = len(executor.calls)
                second = preparer.prepare(
                    profile_path=None,
                    ensure_workload_image=False,
                )

        self.assertEqual(first, second)
        self.assertGreater(first_call_count, 0)
        self.assertEqual(len(executor.calls), first_call_count)

    def test_cloud_hypervisor_prepare_publishes_clh_profile(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            inputs = _inputs(temporary, backend="cloud-hypervisor")
            executor = _FakeExecutor(inputs.bin_dir)
            profile = temporary / "v2-test-profile.json"

            with redirect_stdout(io.StringIO()):
                manifest = ArtifactPreparer(inputs, executor).prepare(
                    profile_path=profile,
                    ensure_workload_image=False,
                )

            manifest_document = json.loads(manifest.read_text(encoding="utf-8"))
            profile_document = json.loads(profile.read_text(encoding="utf-8"))
            base_config = (manifest.parent / "configuration-base.toml").read_text(
                encoding="utf-8"
            )

        self.assertEqual(manifest_document["backend"], "cloud-hypervisor")
        self.assertFalse(manifest_document["otel_export_enabled"])
        self.assertIn("[hypervisor.clh]", base_config)
        self.assertEqual(
            profile_document["environment"]["VIRTUAL_CONTAINER_E2E_BACKENDS"],
            "cloud-hypervisor",
        )
        self.assertEqual(
            profile_document["environment"][
                "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_BACKEND"
            ],
            "cloud-hypervisor",
        )
        self.assertEqual(
            profile_document["name"],
            "openEuler / Kata 3.32 / Cloud Hypervisor",
        )
        config_calls = [
            call
            for call in executor.calls
            if any(Path(value).name == "prepare-stratovirt-config.py" for value in call)
        ]
        self.assertEqual(len(config_calls), 2)
        for call in config_calls:
            self.assertEqual(call[call.index("--backend") + 1], "cloud-hypervisor")
        image_calls = [
            call for call in executor.calls if Path(call[0]).name == "inject-image.sh"
        ]
        self.assertEqual(len(image_calls), 2)
        for call in image_calls:
            self.assertNotIn("--otel-endpoint", call)
            self.assertEqual(call[call.index("--grow-mib") + 1], "128")

    def test_firecracker_artifacts_require_the_guest_system_observer(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            inputs = _inputs(Path(raw_dir), backend="firecracker")

            with self.assertRaisesRegex(
                ValueError,
                "Firecracker artifacts require --with-sandbox-observer",
            ):
                inputs.validate()

    def test_firecracker_jailer_is_a_validated_content_addressed_input(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            inputs = replace(
                _inputs(Path(raw_dir), backend="firecracker"),
                sandbox_observer=True,
            )
            self.assertIsNotNone(inputs.jailer)
            document = build_input_document(inputs)
            original_key = cache_key_for(document)

            assert inputs.jailer is not None
            self.assertEqual(
                document["paths"]["jailer"],
                str(inputs.jailer.resolve()),
            )
            self.assertEqual(
                document["files"]["jailer"],
                hashlib.sha256(inputs.jailer.read_bytes()).hexdigest(),
            )
            inputs.jailer.write_bytes(b"changed jailer")
            inputs.jailer.chmod(0o755)

            self.assertNotEqual(
                original_key,
                cache_key_for(build_input_document(inputs)),
            )

            inputs.jailer.chmod(0o644)
            with self.assertRaisesRegex(ValueError, "jailer must be executable"):
                inputs.validate()

    def test_firecracker_prepare_uses_native_config_without_virtiofs(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            inputs = replace(
                _inputs(temporary, backend="firecracker"),
                sandbox_observer=True,
            )
            executor = _FakeExecutor(inputs.bin_dir)
            profile = temporary / "v2-test-profile.json"

            with redirect_stdout(io.StringIO()):
                manifest = ArtifactPreparer(inputs, executor).prepare(
                    profile_path=profile,
                    ensure_workload_image=False,
                )

            manifest_document = json.loads(manifest.read_text(encoding="utf-8"))
            profile_document = json.loads(profile.read_text(encoding="utf-8"))
            base_config = (manifest.parent / "configuration-base.toml").read_text(
                encoding="utf-8"
            )
            data_config = (manifest.parent / "configuration-data.toml").read_text(
                encoding="utf-8"
            )
            config_calls = [
                call
                for call in executor.calls
                if any(
                    Path(value).name == "prepare-stratovirt-config.py"
                    for value in call
                )
            ]
            image_calls = [
                call
                for call in executor.calls
                if Path(call[0]).name == "inject-image.sh"
            ]

        self.assertEqual(manifest_document["backend"], "firecracker")
        self.assertTrue(manifest_document["sandbox_observer_enabled"])
        self.assertNotIn("virtiofsd", manifest_document["inputs"]["files"])
        self.assertNotIn("virtiofsd", manifest_document["inputs"]["paths"])
        self.assertIn("jailer", manifest_document["inputs"]["files"])
        self.assertEqual(
            manifest_document["inputs"]["paths"]["jailer"],
            str(inputs.jailer),
        )
        self.assertIn("[hypervisor.firecracker]", base_config)
        self.assertIn(f'jailer_path = "{inputs.jailer}"', base_config)
        self.assertIn(
            f'valid_jailer_paths = ["{inputs.jailer}"]',
            base_config,
        )
        self.assertNotIn("virtio_fs_daemon", base_config)
        self.assertIn(f'kernel = "{inputs.data_kernel}"', data_config)
        environment = profile_document["environment"]
        self.assertEqual(
            environment["VIRTUAL_CONTAINER_E2E_BACKENDS"],
            "stratovirt",
        )
        self.assertEqual(environment["VIRTUAL_CONTAINER_E2E_SCOPE"], "contracts")
        self.assertNotIn(
            "VIRTUAL_CONTAINER_E2E_ARTIFACT_MANIFEST",
            environment,
        )
        self.assertNotIn(
            "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_BACKEND",
            environment,
        )
        self.assertNotIn(
            "VIRTUAL_CONTAINER_XIAOO_CONCURRENCY_E2E_ARTIFACT_MANIFEST",
            environment,
        )
        self.assertEqual(
            profile_document["name"],
            "openEuler / Kata 3.32 / Firecracker",
        )
        self.assertEqual(
            environment["EXECUTION_ISOLATION_FIRECRACKER_E2E_BACKEND"],
            "firecracker",
        )
        self.assertEqual(
            environment[
                "EXECUTION_ISOLATION_FIRECRACKER_E2E_ARTIFACT_MANIFEST"
            ],
            str(manifest),
        )
        self.assertNotIn(
            "EXECUTION_ISOLATION_FIRECRACKER_E2E_ARTIFACT_SOURCE_BACKEND",
            environment,
        )
        self.assertEqual(len(config_calls), 2)
        for call in config_calls:
            self.assertEqual(call[call.index("--backend") + 1], "firecracker")
            self.assertNotIn("--virtiofsd", call)
            self.assertEqual(call[call.index("--jailer") + 1], str(inputs.jailer))
        self.assertEqual(len(image_calls), 2)
        for call in image_calls:
            self.assertIn("--with-sandbox-observer", call)
            self.assertEqual(call[call.index("--grow-mib") + 1], "128")

    def test_firecracker_check_only_validates_execution_profile(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            base = _inputs(temporary, backend="firecracker")
            xiaoo = temporary / "xiaoo"
            xiaoo.write_bytes(b"xiaoo\n")
            xiaoo.chmod(0o755)
            archive = temporary / "workload.docker.tar"
            _write_docker_archive(archive, base.workload_image)
            inputs = replace(
                base,
                sandbox_observer=True,
                xiaoo=xiaoo,
                workload_image_archive=archive,
            )
            executor = _FakeExecutor(inputs.bin_dir)
            preparer = ArtifactPreparer(inputs, executor)
            profile = temporary / "v2-test-profile.json"

            with redirect_stdout(io.StringIO()):
                first = preparer.prepare(
                    profile_path=profile,
                    ensure_workload_image=False,
                )
                first_call_count = len(executor.calls)
                second = preparer.prepare(
                    profile_path=profile,
                    check_only=True,
                    ensure_workload_image=False,
                )
            profile_document = json.loads(profile.read_text(encoding="utf-8"))
            image_key = "EXECUTION_ISOLATION_FIRECRACKER_E2E_IMAGE"
            archive_key = (
                "EXECUTION_ISOLATION_FIRECRACKER_E2E_IMAGE_ARCHIVE"
            )
            expected_image = profile_document["environment"][image_key]
            expected_archive = profile_document["environment"][archive_key]

            profile_document["environment"][image_key] = inputs.workload_image
            profile.write_text(json.dumps(profile_document), encoding="utf-8")
            with self.assertRaisesRegex(RuntimeError, "workload image"):
                with redirect_stdout(io.StringIO()):
                    preparer.prepare(
                        profile_path=profile,
                        check_only=True,
                        ensure_workload_image=False,
                    )

            profile_document["environment"][image_key] = expected_image
            profile_document["environment"][archive_key] = str(archive)
            profile.write_text(json.dumps(profile_document), encoding="utf-8")
            with self.assertRaisesRegex(RuntimeError, "image archive"):
                with redirect_stdout(io.StringIO()):
                    preparer.prepare(
                        profile_path=profile,
                        check_only=True,
                        ensure_workload_image=False,
                    )

        self.assertEqual(first, second)
        self.assertNotEqual(expected_image, inputs.workload_image)
        self.assertNotEqual(expected_archive, str(archive))
        self.assertGreater(first_call_count, 0)
        self.assertEqual(len(executor.calls), first_call_count)

    def test_firecracker_workload_archive_uses_devmapper_snapshotter(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            archive = temporary / "workload.tar"
            archive.write_bytes(b"archive")
            inputs = replace(
                _inputs(temporary, backend="firecracker"),
                sandbox_observer=True,
                workload_image_archive=archive,
                image_pull_policy="missing",
            )
            executor = _ImageExecutor(inputs.workload_image)

            ArtifactPreparer(inputs, executor)._ensure_workload_image()

        self.assertEqual(
            executor.calls,
            [
                (
                    "ctr",
                    "-n",
                    "default",
                    "images",
                    "check",
                    "--snapshotter",
                    "devmapper",
                    f"name=={inputs.workload_image}",
                ),
                (
                    "ctr",
                    "-n",
                    "default",
                    "images",
                    "import",
                    "--snapshotter",
                    "devmapper",
                    str(archive),
                ),
                (
                    "ctr",
                    "-n",
                    "default",
                    "images",
                    "check",
                    "--snapshotter",
                    "devmapper",
                    f"name=={inputs.workload_image}",
                ),
            ],
        )

    def test_shared_filesystem_workload_archive_keeps_direct_import(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            archive = temporary / "workload.tar"
            archive.write_bytes(b"archive")
            inputs = replace(
                _inputs(temporary, backend="cloud-hypervisor"),
                workload_image_archive=archive,
                image_pull_policy="missing",
            )
            executor = _ImageExecutor(inputs.workload_image)

            ArtifactPreparer(inputs, executor)._ensure_workload_image()

        self.assertEqual(
            executor.calls,
            [
                (
                    "ctr",
                    "-n",
                    "default",
                    "images",
                    "list",
                    "--quiet",
                    f"name=={inputs.workload_image}",
                ),
                (
                    "ctr",
                    "-n",
                    "default",
                    "images",
                    "import",
                    str(archive),
                ),
            ],
        )

    def test_firecracker_artifact_preinstalls_xiaoo_in_workload_image(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            base = _inputs(temporary, backend="firecracker")
            xiaoo = temporary / "xiaoo"
            xiaoo.write_bytes(b"preinstalled-xiaoo\n")
            xiaoo.chmod(0o755)
            archive = temporary / "workload.docker.tar"
            _write_docker_archive(archive, base.workload_image)
            inputs = replace(
                base,
                sandbox_observer=True,
                xiaoo=xiaoo,
                workload_image_archive=archive,
            )
            profile = temporary / "v2-test-profile.json"

            with redirect_stdout(io.StringIO()):
                manifest = ArtifactPreparer(
                    inputs,
                    _FakeExecutor(inputs.bin_dir),
                ).prepare(
                    profile_path=profile,
                    ensure_workload_image=False,
                )

            manifest_document = json.loads(manifest.read_text(encoding="utf-8"))
            workload = manifest_document["workload_image"]
            prepared_archive = manifest.parent / workload["archive"]
            deployment = DeploymentArtifacts.load(
                manifest,
                bin_dir=inputs.bin_dir,
                expected_backend="firecracker",
                expected_runtime=inputs.runtime,
                require_xiaoo=True,
                require_preinstalled_xiaoo=True,
            )
            profile_environment = json.loads(
                profile.read_text(encoding="utf-8")
            )["environment"]
            layer_payload = _docker_archive_path(
                prepared_archive,
                "opt/actrail-execution/xiaoo-real",
            )
            oci_reference, oci_layer_payload = _oci_archive_path(
                prepared_archive,
                "opt/actrail-execution/xiaoo-real",
            )

        self.assertNotEqual(workload["reference"], inputs.workload_image)
        self.assertEqual(layer_payload, b"preinstalled-xiaoo\n")
        self.assertEqual(oci_reference, workload["reference"])
        self.assertEqual(oci_layer_payload, b"preinstalled-xiaoo\n")
        self.assertEqual(
            deployment.preinstalled_xiaoo_path,
            "/opt/actrail-execution/xiaoo-real",
        )
        self.assertEqual(
            deployment.workload_image_archive,
            prepared_archive.resolve(),
        )
        self.assertEqual(
            deployment.workload_image_archive_sha256,
            workload["archive_sha256"],
        )
        self.assertEqual(
            profile_environment[
                "EXECUTION_ISOLATION_FIRECRACKER_E2E_IMAGE"
            ],
            workload["reference"],
        )
        self.assertEqual(
            profile_environment[
                "EXECUTION_ISOLATION_FIRECRACKER_E2E_IMAGE_ARCHIVE"
            ],
            str(prepared_archive),
        )

    def test_firecracker_workload_archive_rejects_normalized_duplicates(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            archive = temporary / "workload.docker.tar"
            _write_docker_archive(
                archive,
                "docker.io/library/workload:test",
            )
            with tarfile.open(archive, mode="a") as source:
                payload = b"{}"
                alias = tarfile.TarInfo("./manifest.json")
                alias.size = len(payload)
                source.addfile(alias, io.BytesIO(payload))
            xiaoo = temporary / "xiaoo"
            xiaoo.write_bytes(b"xiaoo\n")

            with self.assertRaisesRegex(ValueError, "duplicate Docker archive"):
                prepare_firecracker_workload_image(
                    archive,
                    xiaoo,
                    temporary / "prepared.tar",
                    "a" * 64,
                    source_reference="docker.io/library/workload:test",
                    source_archive_sha256=hashlib.sha256(
                        archive.read_bytes()
                    ).hexdigest(),
                    xiaoo_sha256=hashlib.sha256(xiaoo.read_bytes()).hexdigest(),
                    expected_architecture=_host_oci_architecture(),
                )

    def test_firecracker_workload_archive_rejects_wrong_base_reference(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            archive = temporary / "workload.docker.tar"
            _write_docker_archive(
                archive,
                "docker.io/library/unrelated:test",
            )
            xiaoo = temporary / "xiaoo"
            xiaoo.write_bytes(b"xiaoo\n")

            with self.assertRaisesRegex(ValueError, "base image reference"):
                prepare_firecracker_workload_image(
                    archive,
                    xiaoo,
                    temporary / "prepared.tar",
                    "a" * 64,
                    source_reference="docker.io/library/workload:test",
                    source_archive_sha256=hashlib.sha256(
                        archive.read_bytes()
                    ).hexdigest(),
                    xiaoo_sha256=hashlib.sha256(xiaoo.read_bytes()).hexdigest(),
                    expected_architecture=_host_oci_architecture(),
                )

    def test_firecracker_workload_archive_accepts_docker_familiar_name(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            reference = "docker.io/library/workload:test"
            archive = temporary / "workload.docker.tar"
            _write_docker_archive(
                archive,
                reference,
                docker_reference="workload:test",
            )
            xiaoo = temporary / "xiaoo"
            xiaoo.write_bytes(b"xiaoo\n")

            prepared = prepare_firecracker_workload_image(
                archive,
                xiaoo,
                temporary / "prepared.tar",
                "a" * 64,
                source_reference=reference,
                source_archive_sha256=hashlib.sha256(
                    archive.read_bytes()
                ).hexdigest(),
                xiaoo_sha256=hashlib.sha256(xiaoo.read_bytes()).hexdigest(),
                expected_architecture=_host_oci_architecture(),
            )

        self.assertTrue(prepared.reference.endswith("a" * 64))

    def test_firecracker_workload_archive_rejects_conflicting_oci_name(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            reference = "docker.io/library/workload:test"
            archive = temporary / "workload.docker.tar"
            _write_docker_archive(
                archive,
                reference,
                oci_reference="docker.io/library/unrelated:test",
            )
            xiaoo = temporary / "xiaoo"
            xiaoo.write_bytes(b"xiaoo\n")

            with self.assertRaisesRegex(ValueError, "OCI base image reference"):
                prepare_firecracker_workload_image(
                    archive,
                    xiaoo,
                    temporary / "prepared.tar",
                    "a" * 64,
                    source_reference=reference,
                    source_archive_sha256=hashlib.sha256(
                        archive.read_bytes()
                    ).hexdigest(),
                    xiaoo_sha256=hashlib.sha256(xiaoo.read_bytes()).hexdigest(),
                    expected_architecture=_host_oci_architecture(),
                )

    def test_firecracker_workload_archive_rejects_wrong_platform(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            reference = "docker.io/library/workload:test"
            archive = temporary / "workload.docker.tar"
            expected_architecture = _host_oci_architecture()
            wrong_architecture = (
                "arm64" if expected_architecture == "amd64" else "amd64"
            )
            _write_docker_archive(
                archive,
                reference,
                architecture=wrong_architecture,
            )
            xiaoo = temporary / "xiaoo"
            xiaoo.write_bytes(b"xiaoo\n")

            with self.assertRaisesRegex(ValueError, "platform"):
                prepare_firecracker_workload_image(
                    archive,
                    xiaoo,
                    temporary / "prepared.tar",
                    "a" * 64,
                    source_reference=reference,
                    source_archive_sha256=hashlib.sha256(
                        archive.read_bytes()
                    ).hexdigest(),
                    xiaoo_sha256=hashlib.sha256(xiaoo.read_bytes()).hexdigest(),
                    expected_architecture=expected_architecture,
                )

    def test_firecracker_workload_archive_is_bound_to_hashed_inputs(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            reference = "docker.io/library/workload:test"
            archive = temporary / "workload.docker.tar"
            _write_docker_archive(archive, reference)
            xiaoo = temporary / "xiaoo"
            xiaoo.write_bytes(b"xiaoo\n")
            archive_digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            xiaoo_digest = hashlib.sha256(xiaoo.read_bytes()).hexdigest()

            with self.assertRaisesRegex(ValueError, "source archive digest"):
                prepare_firecracker_workload_image(
                    archive,
                    xiaoo,
                    temporary / "wrong-archive.tar",
                    "a" * 64,
                    source_reference=reference,
                    source_archive_sha256="0" * 64,
                    xiaoo_sha256=xiaoo_digest,
                    expected_architecture=_host_oci_architecture(),
                )
            with self.assertRaisesRegex(ValueError, "xiaoO digest"):
                prepare_firecracker_workload_image(
                    archive,
                    xiaoo,
                    temporary / "wrong-xiaoo.tar",
                    "a" * 64,
                    source_reference=reference,
                    source_archive_sha256=archive_digest,
                    xiaoo_sha256="0" * 64,
                    expected_architecture=_host_oci_architecture(),
                )

    def test_rederived_firecracker_archive_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            reference = "docker.io/library/workload:test"
            archive = temporary / "workload.docker.tar"
            _write_docker_archive(archive, reference)
            xiaoo = temporary / "xiaoo"
            xiaoo.write_bytes(b"xiaoo\n")
            xiaoo_digest = hashlib.sha256(xiaoo.read_bytes()).hexdigest()
            first = prepare_firecracker_workload_image(
                archive,
                xiaoo,
                temporary / "first.tar",
                "a" * 64,
                source_reference=reference,
                source_archive_sha256=hashlib.sha256(
                    archive.read_bytes()
                ).hexdigest(),
                xiaoo_sha256=xiaoo_digest,
                expected_architecture=_host_oci_architecture(),
            )
            with self.assertRaisesRegex(ValueError, "generated blob"):
                prepare_firecracker_workload_image(
                    first.archive,
                    xiaoo,
                    temporary / "second.tar",
                    "b" * 64,
                    source_reference=first.reference,
                    source_archive_sha256=first.archive_sha256,
                    xiaoo_sha256=xiaoo_digest,
                    expected_architecture=_host_oci_architecture(),
                )

    def test_firecracker_xiaoo_requires_a_workload_image_archive(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            xiaoo = temporary / "xiaoo"
            xiaoo.write_bytes(b"xiaoo\n")
            xiaoo.chmod(0o755)
            inputs = replace(
                _inputs(temporary, backend="firecracker"),
                sandbox_observer=True,
                xiaoo=xiaoo,
            )

            with self.assertRaisesRegex(
                ValueError,
                "workload-image-archive.*preinstalled",
            ):
                inputs.validate()

    def test_firecracker_import_checks_the_derived_image_reference(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            archive = temporary / "derived-workload.tar"
            archive.write_bytes(b"archive")
            inputs = replace(
                _inputs(temporary, backend="firecracker"),
                sandbox_observer=True,
                image_pull_policy="missing",
            )
            derived_reference = "example.test/firecracker-workload:derived"
            executor = _ImageExecutor(derived_reference)

            ArtifactPreparer(inputs, executor)._ensure_workload_image(
                reference=derived_reference,
                archive=archive,
                archive_sha256=hashlib.sha256(
                    archive.read_bytes()
                ).hexdigest(),
            )

        imported = next(
            call
            for call in executor.calls
            if call[3:5] == ("images", "import")
        )
        self.assertNotEqual(Path(imported[-1]), archive)
        self.assertFalse(Path(imported[-1]).exists())
        self.assertEqual(
            executor.calls[-1][-1],
            f"name=={derived_reference}",
        )

    def test_firecracker_workload_pull_uses_devmapper_snapshotter(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            inputs = replace(
                _inputs(Path(raw_dir), backend="firecracker"),
                sandbox_observer=True,
                image_pull_policy="missing",
            )
            executor = _ImageExecutor(inputs.workload_image)

            ArtifactPreparer(inputs, executor)._ensure_workload_image()

        pull = next(
            call
            for call in executor.calls
            if call[3:5] == ("images", "pull")
        )
        self.assertEqual(
            pull[3:],
            (
                "images",
                "pull",
                "--snapshotter",
                "devmapper",
                inputs.workload_image,
            ),
        )
        self.assertEqual(
            executor.calls[-1][3:],
            (
                "images",
                "check",
                "--snapshotter",
                "devmapper",
                f"name=={inputs.workload_image}",
            ),
        )
        for call in executor.calls:
            if call[3:5] == ("images", "list"):
                self.assertNotIn("--snapshotter", call)

    def test_firecracker_never_policy_rejects_complete_but_unpacked_false(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            inputs = replace(
                _inputs(Path(raw_dir), backend="firecracker"),
                sandbox_observer=True,
            )
            executor = _ImageExecutor(
                inputs.workload_image,
                present=True,
                unpacked=False,
            )

            with self.assertRaisesRegex(
                RuntimeError,
                "not unpacked for snapshotter devmapper.*pull policy is never",
            ):
                ArtifactPreparer(inputs, executor)._ensure_workload_image()

        self.assertNotIn("--quiet", executor.calls[0])

    def test_firecracker_import_must_finish_unpacked_in_devmapper(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            archive = temporary / "workload.tar"
            archive.write_bytes(b"archive")
            inputs = replace(
                _inputs(temporary, backend="firecracker"),
                sandbox_observer=True,
                workload_image_archive=archive,
                image_pull_policy="missing",
            )
            executor = _ImageExecutor(
                inputs.workload_image,
                prepare_unpacked=False,
            )

            with self.assertRaisesRegex(
                RuntimeError,
                "import completed.*not unpacked.*devmapper",
            ):
                ArtifactPreparer(inputs, executor)._ensure_workload_image()

        self.assertEqual(
            [call[3:5] for call in executor.calls],
            [("images", "check"), ("images", "import"), ("images", "check")],
        )

    def test_vsock_prepare_publishes_vsock_guest_images(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            inputs = replace(
                _inputs(Path(raw_dir), backend="cloud-hypervisor"),
                egress_mode="vsock-bridge",
                otel_endpoint="http://127.0.0.1:14318/v1/traces",
            )
            executor = _FakeExecutor(inputs.bin_dir)

            with redirect_stdout(io.StringIO()):
                manifest = ArtifactPreparer(inputs, executor).prepare(
                    profile_path=None,
                    ensure_workload_image=False,
                )

            manifest_document = json.loads(manifest.read_text(encoding="utf-8"))
            deployment = DeploymentArtifacts.load(
                manifest,
                bin_dir=inputs.bin_dir,
                expected_backend=inputs.backend,
                expected_runtime=inputs.runtime,
            )
            image_calls = [
                call
                for call in executor.calls
                if Path(call[0]).name == "inject-image.sh"
            ]

        self.assertEqual(manifest_document["egress_mode"], "vsock-bridge")
        self.assertTrue(manifest_document["otel_export_enabled"])
        self.assertEqual(deployment.egress_mode, "vsock-bridge")
        self.assertTrue(deployment.otel_export_enabled)
        self.assertEqual(len(image_calls), 2)
        for call in image_calls:
            self.assertEqual(
                call[call.index("--egress-mode") + 1],
                "vsock-bridge",
            )

    def test_sudo_prepare_returns_checkout_outputs_to_invoking_user(self) -> None:
        deploy_uid = 1002
        deploy_gid = 1002
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            inputs = _inputs(temporary)
            executor = _FakeExecutor(inputs.bin_dir)
            profile = temporary / "v2-test-profile.json"
            environment = {
                "SUDO_UID": str(deploy_uid),
                "SUDO_GID": str(deploy_gid),
                "SUDO_USER": "developer",
            }

            with patch.dict(os.environ, environment, clear=False):
                with patch(
                    "v2_artifacts_support.metadata.os.geteuid",
                    return_value=0,
                ):
                    with patch(
                        "v2_artifacts_support.metadata.chown_tree"
                    ) as chown_tree:
                        with redirect_stdout(io.StringIO()):
                            manifest = ArtifactPreparer(inputs, executor).prepare(
                                profile_path=profile,
                                ensure_workload_image=False,
                            )

        chown_tree.assert_any_call(
            inputs.output_root,
            deploy_uid,
            deploy_gid,
            recursive=False,
        )
        chown_tree.assert_any_call(
            manifest.parent,
            deploy_uid,
            deploy_gid,
            recursive=True,
        )
        chown_tree.assert_any_call(
            profile,
            deploy_uid,
            deploy_gid,
            recursive=False,
        )

    def test_sudo_prepare_restores_artifact_owner_when_image_check_fails(self) -> None:
        deploy_uid = 1002
        deploy_gid = 1002
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            inputs = _inputs(Path(raw_dir))
            executor = _FailingImageExecutor(inputs.bin_dir)
            environment = {
                "SUDO_UID": str(deploy_uid),
                "SUDO_GID": str(deploy_gid),
                "SUDO_USER": "developer",
            }

            with patch.dict(os.environ, environment, clear=False):
                with patch(
                    "v2_artifacts_support.metadata.os.geteuid",
                    return_value=0,
                ):
                    with patch(
                        "v2_artifacts_support.metadata.chown_tree"
                    ) as chown_tree:
                        with self.assertRaisesRegex(
                            RuntimeError,
                            "containerd unavailable",
                        ):
                            with redirect_stdout(io.StringIO()):
                                ArtifactPreparer(inputs, executor).prepare(
                                    profile_path=None,
                                )

        artifact_calls = [
            call
            for call in chown_tree.call_args_list
            if call.kwargs.get("recursive") is True
        ]
        self.assertEqual(len(artifact_calls), 1)


def _inputs(root: Path, *, backend: str = "stratovirt") -> PreparationInputs:
    repo = root / "repo"
    bin_dir = repo / "target/release"
    bin_dir.mkdir(parents=True)
    for name in (
        "actraild",
        "actrailctl",
        "actrail-sb",
        "actrail-vsock-gateway",
        "actrailviewer",
        "libactrail_tls_payload_probe_sync.so",
    ):
        (bin_dir / name).write_bytes(name.encode())
        if name in {"actrail-sb", "actrail-vsock-gateway"}:
            (bin_dir / name).chmod(0o755)

    plugin_dir = repo / "examples/plugins/builtin/sandbox-resource-alert"
    plugin_dir.mkdir(parents=True)
    for name in (
        "sandbox-resource-alert.plugin.toml",
        "sandbox-resource-alert.config.json",
        "sandbox-resource-alert.config.v1.schema.json",
    ):
        (plugin_dir / name).write_text(name + "\n", encoding="utf-8")

    sources = root / "sources"
    sources.mkdir()
    paths = {}
    hypervisor_name = {
        "stratovirt": "stratovirt",
        "cloud-hypervisor": "cloud-hypervisor",
        "firecracker": "firecracker",
    }[backend]
    source_names = [
        "base.toml",
        "data.toml",
        "base.img",
        "data.img",
        "base-kernel",
        "data-kernel",
        hypervisor_name,
    ]
    if backend != "firecracker":
        source_names.append("virtiofsd")
    else:
        source_names.append("jailer")
    for name in source_names:
        path = sources / name
        path.write_bytes(name.encode())
        if name in {
            "stratovirt",
            "cloud-hypervisor",
            "firecracker",
            "jailer",
            "virtiofsd",
        }:
            path.chmod(0o755)
        paths[name] = path
    observer_kernel_config = "\n".join(
        (
            "CONFIG_BPF=y",
            "CONFIG_BPF_SYSCALL=y",
            "CONFIG_BPF_JIT=y",
            "CONFIG_BPF_EVENTS=y",
            "CONFIG_DEBUG_INFO_BTF=y",
            "CONFIG_FTRACE=y",
            "CONFIG_FTRACE_SYSCALLS=y",
            "CONFIG_KPROBES=y",
            "CONFIG_KPROBE_EVENTS=y",
            "CONFIG_PERF_EVENTS=y",
            "CONFIG_TRACEPOINTS=y",
            "CONFIG_TRACING=y",
            "CONFIG_UPROBES=y",
            "CONFIG_UPROBE_EVENTS=y",
        )
    )
    for kernel_name in ("base-kernel", "data-kernel"):
        Path(f"{paths[kernel_name]}.config").write_text(
            observer_kernel_config + "\n",
            encoding="utf-8",
        )
    return PreparationInputs(
        repo=repo,
        bin_dir=bin_dir,
        output_root=repo / "local/kata/artifacts",
        backend=backend,
        runtime="io.containerd.kata332.v2",
        kata_prefix=Path("/opt/kata"),
        base_config_source=paths["base.toml"],
        data_config_source=paths["data.toml"],
        base_image_source=paths["base.img"],
        data_image_source=paths["data.img"],
        hypervisor=paths[hypervisor_name],
        base_kernel=paths["base-kernel"],
        data_kernel=paths["data-kernel"],
        virtiofsd=paths.get("virtiofsd"),
        xiaoo=None,
        workload_image="example.test/workload:24.09",
        workload_image_archive=None,
        image_pull_policy="never",
        otel_endpoint=None,
        socket_gid=39000,
        data_vcpus=2,
    )


class _FakeExecutor:
    def __init__(self, bin_dir: Path) -> None:
        self.bin_dir = bin_dir
        self.calls: list[tuple[str, ...]] = []

    def run(
        self,
        command,
        *,
        environment=None,
        capture=False,
    ) -> subprocess.CompletedProcess[str]:
        argv = tuple(str(value) for value in command)
        self.calls.append(argv)
        executable = Path(argv[0]).name
        if executable == "prepare-guest-bundle.sh":
            assert environment is not None
            output = Path(environment["BUNDLE_DIR"])
            output.mkdir()
            for name in (
                "actraild",
                "actrailctl",
                "actrail-sb",
                "actrailviewer",
                "libactrail_tls_payload_probe_sync.so",
            ):
                shutil.copy2(self.bin_dir / name, output / name)
            _write_manifest(output)
        elif executable == "prepare-bundle.sh":
            output = Path(argv[argv.index("--output") + 1])
            guest = Path(argv[argv.index("--guest-bundle") + 1])
            (output / "bin").mkdir(parents=True)
            shutil.copy2(guest / "actrailctl", output / "bin/actrailctl")
            _write_manifest(output)
        elif executable == "inject-image.sh":
            source = Path(argv[argv.index("--source-image") + 1])
            output = Path(argv[argv.index("--output-image") + 1])
            shutil.copy2(source, output)
        elif any(
            Path(value).name == "prepare-stratovirt-config.py"
            for value in argv
        ):
            backend = (
                argv[argv.index("--backend") + 1]
                if "--backend" in argv
                else "stratovirt"
            )
            section = {
                "stratovirt": "stratovirt",
                "cloud-hypervisor": "clh",
                "firecracker": "firecracker",
            }[backend]
            output = Path(argv[argv.index("--output") + 1])
            image = argv[argv.index("--image-config-path") + 1]
            hypervisor = argv[argv.index("--hypervisor") + 1]
            kernel = argv[argv.index("--kernel") + 1]
            content = (
                f"[hypervisor.{section}]\n"
                f'path = "{hypervisor}"\n'
                f'kernel = "{kernel}"\n'
                f'image = "{image}"\n'
            )
            if "--virtiofsd" in argv:
                virtiofsd = argv[argv.index("--virtiofsd") + 1]
                content += f'virtio_fs_daemon = "{virtiofsd}"\n'
            if "--jailer" in argv:
                jailer = argv[argv.index("--jailer") + 1]
                content += (
                    f'jailer_path = "{jailer}"\n'
                    f'valid_jailer_paths = ["{jailer}"]\n'
                )
            output.write_text(content, encoding="utf-8")
        else:
            raise AssertionError(f"unexpected command: {argv}")
        return subprocess.CompletedProcess(argv, 0, "", "")


class _FailingImageExecutor(_FakeExecutor):
    def run(self, command, *, environment=None, capture=False):
        if Path(str(command[0])).name == "ctr":
            raise RuntimeError("containerd unavailable")
        return super().run(
            command,
            environment=environment,
            capture=capture,
        )


class _ImageExecutor:
    def __init__(
        self,
        reference: str,
        *,
        present: bool = False,
        complete: bool = True,
        unpacked: bool = True,
        prepare_unpacked: bool = True,
    ) -> None:
        self.reference = reference
        self.present = present
        self.complete = complete
        self.unpacked = unpacked
        self.prepare_unpacked = prepare_unpacked
        self.calls: list[tuple[str, ...]] = []

    def run(self, command, *, environment=None, capture=False):
        del environment, capture
        argv = tuple(str(value) for value in command)
        self.calls.append(argv)
        if argv[3:5] == ("images", "check"):
            if "--quiet" in argv:
                # Model the containerd 1.6 bug: quiet ignores unpacked state.
                stdout = (
                    f"{self.reference}\n"
                    if self.present and self.complete
                    else ""
                )
                return subprocess.CompletedProcess(argv, 0, stdout, "")
            stdout = "REF TYPE DIGEST STATUS SIZE UNPACKED\n"
            if self.present:
                status = "complete (3/3)" if self.complete else "incomplete (2/3)"
                stdout += (
                    f"{self.reference} application/vnd.oci.image.manifest.v1+json "
                    f"sha256:abcd {status} 233.9MiB/233.9MiB "
                    f"{str(self.unpacked).lower()}\n"
                )
            return subprocess.CompletedProcess(argv, 0, stdout, "")
        if argv[3:5] in {("images", "import"), ("images", "pull")}:
            self.present = True
            self.complete = True
            self.unpacked = self.prepare_unpacked
        return subprocess.CompletedProcess(argv, 0, "", "")


def _host_oci_architecture() -> str:
    machine = os.uname().machine.lower()
    return {
        "aarch64": "arm64",
        "x86_64": "amd64",
    }.get(machine, machine)


def _write_docker_archive(
    path: Path,
    reference: str,
    *,
    docker_reference: str | None = None,
    oci_reference: str | None = None,
    architecture: str | None = None,
) -> None:
    if architecture is None:
        architecture = _host_oci_architecture()
    if docker_reference is None:
        docker_reference = reference
    if oci_reference is None:
        oci_reference = reference
    layer_buffer = io.BytesIO()
    with tarfile.open(fileobj=layer_buffer, mode="w") as layer:
        payload = b"base-image\n"
        info = tarfile.TarInfo("etc/base-image")
        info.size = len(payload)
        info.mode = 0o644
        layer.addfile(info, io.BytesIO(payload))
    layer_payload = layer_buffer.getvalue()
    layer_digest = hashlib.sha256(layer_payload).hexdigest()
    layer_path = f"blobs/sha256/{layer_digest}"
    config = {
        "architecture": architecture,
        "os": "linux",
        "config": {"Cmd": ["/bin/sh"]},
        "rootfs": {
            "type": "layers",
            "diff_ids": [f"sha256:{layer_digest}"],
        },
        "history": [{"created_by": "AcTrail test fixture"}],
    }
    config_payload = json.dumps(
        config,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    config_digest = hashlib.sha256(config_payload).hexdigest()
    config_path = f"blobs/sha256/{config_digest}"
    repository, tag = docker_reference.rsplit(":", 1)
    manifest_payload = json.dumps(
        [
            {
                "Config": config_path,
                "RepoTags": [docker_reference],
                "Layers": [layer_path],
                "LayerSources": {
                    f"sha256:{layer_digest}": {
                        "mediaType": "application/vnd.oci.image.layer.v1.tar",
                        "size": len(layer_payload),
                        "digest": f"sha256:{layer_digest}",
                    }
                },
            }
        ],
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    repositories_payload = json.dumps(
        {repository: {tag: layer_digest}},
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    oci_manifest_payload = json.dumps(
        {
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "config": {
                "mediaType": "application/vnd.oci.image.config.v1+json",
                "digest": f"sha256:{config_digest}",
                "size": len(config_payload),
                "platform": {"architecture": architecture, "os": "linux"},
            },
            "layers": [
                {
                    "mediaType": "application/vnd.oci.image.layer.v1.tar",
                    "digest": f"sha256:{layer_digest}",
                    "size": len(layer_payload),
                }
            ],
        },
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    oci_manifest_digest = hashlib.sha256(oci_manifest_payload).hexdigest()
    index_payload = json.dumps(
        {
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "manifests": [
                {
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "digest": f"sha256:{oci_manifest_digest}",
                    "size": len(oci_manifest_payload),
                    "annotations": {
                        "io.containerd.image.name": oci_reference,
                        "org.opencontainers.image.ref.name": (
                            oci_reference.rsplit(":", 1)[1]
                        ),
                    },
                    "platform": {"architecture": architecture, "os": "linux"},
                }
            ],
        },
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    oci_layout_payload = b'{"imageLayoutVersion":"1.0.0"}'
    with tarfile.open(path, mode="w") as archive:
        for name, payload in (
            (config_path, config_payload),
            (layer_path, layer_payload),
            (f"blobs/sha256/{oci_manifest_digest}", oci_manifest_payload),
            ("index.json", index_payload),
            ("oci-layout", oci_layout_payload),
            ("manifest.json", manifest_payload),
            ("repositories", repositories_payload),
        ):
            info = tarfile.TarInfo(name)
            info.size = len(payload)
            info.mode = 0o644
            archive.addfile(info, io.BytesIO(payload))


def _docker_archive_path(archive_path: Path, expected: str) -> bytes:
    with tarfile.open(archive_path, mode="r") as archive:
        manifest_source = archive.extractfile("manifest.json")
        assert manifest_source is not None
        manifest = json.load(manifest_source)[0]
        for layer_path in reversed(manifest["Layers"]):
            layer_source = archive.extractfile(layer_path)
            assert layer_source is not None
            with tarfile.open(fileobj=layer_source, mode="r") as layer:
                try:
                    payload = layer.extractfile(expected)
                except KeyError:
                    continue
                assert payload is not None
                return payload.read()
    raise AssertionError(f"Docker archive omitted {expected}")


def _oci_archive_path(archive_path: Path, expected: str) -> tuple[str, bytes]:
    with tarfile.open(archive_path, mode="r") as archive:
        index_source = archive.extractfile("index.json")
        assert index_source is not None
        index = json.load(index_source)
        descriptor = index["manifests"][0]
        manifest_digest = descriptor["digest"].removeprefix("sha256:")
        manifest_source = archive.extractfile(
            f"blobs/sha256/{manifest_digest}"
        )
        assert manifest_source is not None
        manifest_payload = manifest_source.read()
        assert hashlib.sha256(manifest_payload).hexdigest() == manifest_digest
        assert len(manifest_payload) == descriptor["size"]
        manifest = json.loads(manifest_payload)
        config_descriptor = manifest["config"]
        config_digest = config_descriptor["digest"].removeprefix("sha256:")
        config_source = archive.extractfile(f"blobs/sha256/{config_digest}")
        assert config_source is not None
        config_payload = config_source.read()
        assert hashlib.sha256(config_payload).hexdigest() == config_digest
        assert len(config_payload) == config_descriptor["size"]
        for layer_descriptor in reversed(manifest["layers"]):
            layer_digest = layer_descriptor["digest"].removeprefix("sha256:")
            layer_source = archive.extractfile(f"blobs/sha256/{layer_digest}")
            assert layer_source is not None
            layer_payload = layer_source.read()
            assert hashlib.sha256(layer_payload).hexdigest() == layer_digest
            assert len(layer_payload) == layer_descriptor["size"]
            with tarfile.open(fileobj=io.BytesIO(layer_payload), mode="r") as layer:
                try:
                    payload = layer.extractfile(expected)
                except KeyError:
                    continue
                assert payload is not None
                return (
                    descriptor["annotations"]["io.containerd.image.name"],
                    payload.read(),
                )
    raise AssertionError(f"OCI archive omitted {expected}")


def _write_manifest(directory: Path) -> None:
    lines = []
    for path in sorted(directory.rglob("*")):
        if path.is_file() and path.name != "MANIFEST.sha256":
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            relative = path.relative_to(directory).as_posix()
            lines.append(f"{digest}  ./{relative}\n")
    (directory / "MANIFEST.sha256").write_text("".join(lines), encoding="utf-8")


if __name__ == "__main__":
    unittest.main()
