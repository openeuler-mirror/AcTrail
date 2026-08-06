from __future__ import annotations

import json
import hashlib
import io
import os
import shutil
import subprocess
import sys
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
    write_test_profile,
)


class ArtifactPreparerTest(unittest.TestCase):
    def test_cache_key_is_stable_until_an_input_changes(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            inputs = _inputs(Path(raw_dir))
            first = cache_key_for(build_input_document(inputs))
            second = cache_key_for(build_input_document(inputs))
            self.assertEqual(first, second)

            inputs.data_image_source.write_bytes(b"changed data image")
            changed = cache_key_for(build_input_document(inputs))
            self.assertNotEqual(first, changed)

    def test_otel_endpoint_is_required_and_participates_in_cache_key(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            inputs = _inputs(Path(raw_dir))
            first = cache_key_for(build_input_document(inputs))
            changed_inputs = replace(
                inputs,
                otel_endpoint="https://collector.example:4318/v1/traces",
            )
            changed = cache_key_for(build_input_document(changed_inputs))

            self.assertNotEqual(first, changed)
            with self.assertRaisesRegex(
                ValueError,
                "Guest OTLP/HTTP endpoint must not be empty",
            ):
                replace(inputs, otel_endpoint="").validate()

    def test_profile_format_two_uses_manifest_as_the_path_source(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prepare.") as raw_dir:
            temporary = Path(raw_dir)
            inputs = _inputs(temporary)
            profile = temporary / "v2-test-profile.json"
            manifest = temporary / ("a" * 64) / "manifest.json"
            manifest.parent.mkdir()
            manifest.write_text("{}", encoding="utf-8")

            write_test_profile(inputs, manifest, profile)
            document = json.loads(profile.read_text(encoding="utf-8"))

        self.assertEqual(document["format"], 2)
        environment = document["environment"]
        self.assertEqual(
            environment["VIRTUAL_CONTAINER_E2E_ARTIFACT_MANIFEST"],
            str(manifest),
        )
        self.assertEqual(environment["VIRTUAL_CONTAINER_E2E_SCOPE"], "auto")
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
            self.assertEqual(
                call[call.index("--otel-endpoint") + 1],
                inputs.otel_endpoint,
            )
            self.assertEqual(call[call.index("--grow-mib") + 1], "128")

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
                with patch("v2_artifacts.os.geteuid", return_value=0):
                    with patch("v2_artifacts._chown_tree") as chown_tree:
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
                with patch("v2_artifacts.os.geteuid", return_value=0):
                    with patch("v2_artifacts._chown_tree") as chown_tree:
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
        "actrailviewer",
        "libactrail_tls_payload_probe_sync.so",
    ):
        (bin_dir / name).write_bytes(name.encode())

    sources = root / "sources"
    sources.mkdir()
    paths = {}
    for name in (
        "base.toml",
        "data.toml",
        "base.img",
        "data.img",
        "base-kernel",
        "data-kernel",
        "cloud-hypervisor" if backend == "cloud-hypervisor" else "stratovirt",
        "virtiofsd",
    ):
        path = sources / name
        path.write_bytes(name.encode())
        if name in {"stratovirt", "cloud-hypervisor", "virtiofsd"}:
            path.chmod(0o755)
        paths[name] = path
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
        hypervisor=paths[
            "cloud-hypervisor" if backend == "cloud-hypervisor" else "stratovirt"
        ],
        base_kernel=paths["base-kernel"],
        data_kernel=paths["data-kernel"],
        virtiofsd=paths["virtiofsd"],
        xiaoo=None,
        workload_image="example.test/workload:24.09",
        workload_image_archive=None,
        image_pull_policy="never",
        otel_endpoint="http://192.0.2.10:4318/v1/traces",
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
            section = "clh" if backend == "cloud-hypervisor" else "stratovirt"
            output = Path(argv[argv.index("--output") + 1])
            image = argv[argv.index("--image-config-path") + 1]
            hypervisor = argv[argv.index("--hypervisor") + 1]
            kernel = argv[argv.index("--kernel") + 1]
            virtiofsd = argv[argv.index("--virtiofsd") + 1]
            output.write_text(
                f"[hypervisor.{section}]\n"
                f'path = "{hypervisor}"\n'
                f'kernel = "{kernel}"\n'
                f'image = "{image}"\n'
                f'virtio_fs_daemon = "{virtiofsd}"\n',
                encoding="utf-8",
            )
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
