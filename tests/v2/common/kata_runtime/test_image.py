from __future__ import annotations

import hashlib
import tempfile
import unittest
from collections.abc import Sequence
from pathlib import Path

from tests.v2.common.kata_runtime.factory import KataRequirementsBuilder
from tests.v2.common.kata_runtime.image import (
    ContainerdImage,
    PullPolicy,
    containerd_image_check_ready,
    firecracker_workload_reference,
)
from tests.v2.common.kata_runtime.process import CommandResult
from tests.v2.common.kata_runtime.requirements import (
    PreparePolicy,
    RequirementCheck,
    ResolvedImage,
)


class ContainerdImageRunner:
    def __init__(
        self,
        reference: str,
        *,
        present: bool,
        complete: bool = True,
        unpacked: bool = True,
    ) -> None:
        self.reference = reference
        self.present = present
        self.complete = complete
        self.unpacked = unpacked
        self.commands: list[tuple[str, ...]] = []
        self.imported_archive: bytes | None = None

    def run(
        self,
        argv: Sequence[str],
        *,
        timeout: float | None = None,
    ) -> CommandResult:
        command = tuple(argv)
        self.commands.append(command)
        del timeout
        if command[3:5] == ("images", "list") and "--quiet" in command:
            output = f"{self.reference}\n" if self.present else ""
            return CommandResult(command, 0, output, "")
        if command[3:5] == ("images", "check"):
            if "--quiet" in command:
                # containerd 1.6 prints complete refs even when UNPACKED=false.
                output = (
                    f"{self.reference}\n"
                    if self.present and self.complete
                    else ""
                )
                return CommandResult(command, 0, output, "")
            output = "REF TYPE DIGEST STATUS SIZE UNPACKED\n"
            if self.present:
                status = "complete (3/3)" if self.complete else "incomplete (2/3)"
                output += (
                    f"{self.reference} application/vnd.oci.image.manifest.v1+json "
                    f"sha256:abcd {status} 233.9MiB/233.9MiB "
                    f"{str(self.unpacked).lower()}\n"
                )
            return CommandResult(command, 0, output, "")
        if command[3:5] == ("images", "list"):
            output = (
                "REF TYPE DIGEST SIZE PLATFORMS LABELS\n"
                f"{self.reference} application/vnd.oci.image.manifest.v1+json "
                "sha256:abcd 233.9MiB linux/arm64 -\n"
            )
            return CommandResult(command, 0, output, "")
        if command[3:5] == ("images", "import"):
            self.imported_archive = Path(command[-1]).read_bytes()
            self.present = True
            self.complete = True
            self.unpacked = True
            return CommandResult(command, 0, "imported\n", "")
        if command[3:5] == ("images", "pull"):
            self.present = True
            self.complete = True
            self.unpacked = True
            return CommandResult(command, 0, "pulled\n", "")
        return CommandResult(command, 1, "", f"unexpected command: {command}")


class ContainerdImageTest(unittest.TestCase):
    def test_firecracker_workload_reference_requires_canonical_cache_key(
        self,
    ) -> None:
        cache_key = "a" * 64
        self.assertEqual(
            firecracker_workload_reference(cache_key),
            "docker.io/library/actrail-firecracker-workload:actrail-"
            + cache_key,
        )
        for invalid in (
            "",
            "a" * 63,
            "a" * 65,
            "A" * 64,
            "../" + "a" * 64,
        ):
            with self.subTest(invalid=invalid):
                with self.assertRaisesRegex(ValueError, "cache key"):
                    firecracker_workload_reference(invalid)

    def test_missing_image_imports_offline_archive_and_returns_digest(self) -> None:
        reference = "docker.io/library/actrail-openeuler-workload:24.09"
        runner = ContainerdImageRunner(reference, present=False)
        with tempfile.TemporaryDirectory(prefix="actrail-image-test.") as raw_dir:
            archive = Path(raw_dir) / "workload.tar"
            archive.write_bytes(b"fixture")
            image = ContainerdImage(
                reference=reference,
                runner=runner,
                namespace="default",
                pull_policy=PullPolicy.MISSING,
                archive=archive,
            )

            resolved = image.ensure(PreparePolicy.MISSING)

        self.assertEqual(resolved.reference, reference)
        self.assertEqual(resolved.digest, "sha256:abcd")
        self.assertEqual(
            [command[3:5] for command in runner.commands].count(
                ("images", "import")
            ),
            1,
        )

    def test_digest_bound_archive_import_uses_verified_private_snapshot(
        self,
    ) -> None:
        reference = "docker.io/library/actrail-openeuler-workload:24.09"
        runner = ContainerdImageRunner(reference, present=False)
        with tempfile.TemporaryDirectory(prefix="actrail-image-test.") as raw_dir:
            archive = Path(raw_dir) / "workload.tar"
            payload = b"verified archive"
            archive.write_bytes(payload)
            image = ContainerdImage(
                reference=reference,
                runner=runner,
                pull_policy=PullPolicy.MISSING,
                archive=archive,
                archive_sha256=hashlib.sha256(payload).hexdigest(),
            )

            image.ensure(PreparePolicy.MISSING)
            import_command = next(
                command
                for command in runner.commands
                if command[3:5] == ("images", "import")
            )

        self.assertEqual(runner.imported_archive, payload)
        self.assertNotEqual(Path(import_command[-1]), archive)
        self.assertFalse(Path(import_command[-1]).exists())

    def test_digest_bound_archive_rejects_changed_bytes_before_import(
        self,
    ) -> None:
        reference = "docker.io/library/actrail-openeuler-workload:24.09"
        runner = ContainerdImageRunner(reference, present=False)
        with tempfile.TemporaryDirectory(prefix="actrail-image-test.") as raw_dir:
            archive = Path(raw_dir) / "workload.tar"
            archive.write_bytes(b"changed archive")
            image = ContainerdImage(
                reference=reference,
                runner=runner,
                pull_policy=PullPolicy.MISSING,
                archive=archive,
                archive_sha256=hashlib.sha256(b"expected archive").hexdigest(),
            )

            with self.assertRaisesRegex(RuntimeError, "digest mismatch"):
                image.ensure(PreparePolicy.MISSING)

        self.assertFalse(
            any(
                command[3:5] == ("images", "import")
                for command in runner.commands
            )
        )

    def test_snapshotter_check_and_import_do_not_flag_images_list(self) -> None:
        reference = "docker.io/library/actrail-openeuler-workload:24.09"
        runner = ContainerdImageRunner(reference, present=False)
        with tempfile.TemporaryDirectory(prefix="actrail-image-test.") as raw_dir:
            archive = Path(raw_dir) / "workload.tar"
            archive.write_bytes(b"fixture")
            image = ContainerdImage(
                reference=reference,
                runner=runner,
                snapshotter="devmapper",
                pull_policy=PullPolicy.MISSING,
                archive=archive,
            )

            image.ensure(PreparePolicy.MISSING)

        self.assertEqual(
            runner.commands[0][3:],
            (
                "images",
                "check",
                "--snapshotter",
                "devmapper",
                f"name=={reference}",
            ),
        )
        self.assertEqual(
            runner.commands[1][3:],
            (
                "images",
                "import",
                "--snapshotter",
                "devmapper",
                str(archive),
            ),
        )
        self.assertEqual(
            runner.commands[-1][3:],
            ("images", "list", f"name=={reference}"),
        )

    def test_containerd_16_quiet_complete_ref_does_not_mask_unpacked_false(
        self,
    ) -> None:
        reference = "docker.io/library/actrail-openeuler-workload:24.09"
        runner = ContainerdImageRunner(
            reference,
            present=True,
            complete=True,
            unpacked=False,
        )
        image = ContainerdImage(
            reference=reference,
            runner=runner,
            snapshotter="devmapper",
            pull_policy=PullPolicy.NEVER,
        )

        with self.assertRaisesRegex(
            RuntimeError,
            "not unpacked for snapshotter devmapper.*pull policy is never",
        ):
            image.ensure(PreparePolicy.MISSING)

        self.assertNotIn("--quiet", runner.commands[0])
        self.assertFalse(containerd_image_check_ready(f"{reference}\n", reference))

    def test_snapshotter_cache_hit_requires_complete_unpacked_row(self) -> None:
        reference = "docker.io/library/actrail-openeuler-workload:24.09"
        incomplete = (
            "REF TYPE DIGEST STATUS SIZE UNPACKED\n"
            f"{reference} application/vnd.oci.image.manifest.v1+json "
            "sha256:abcd incomplete (2/3) 2MiB/3MiB true\n"
        )
        complete = incomplete.replace("incomplete (2/3)", "complete (3/3)")

        self.assertFalse(containerd_image_check_ready(incomplete, reference))
        self.assertTrue(containerd_image_check_ready(complete, reference))

    def test_snapshotter_is_used_to_pull_image(self) -> None:
        reference = "docker.io/library/actrail-openeuler-workload:24.09"
        runner = ContainerdImageRunner(reference, present=False)
        image = ContainerdImage(
            reference=reference,
            runner=runner,
            snapshotter="devmapper",
            pull_policy=PullPolicy.MISSING,
        )

        image.ensure(PreparePolicy.MISSING)

        self.assertEqual(
            runner.commands[1][3:],
            (
                "images",
                "pull",
                "--snapshotter",
                "devmapper",
                reference,
            ),
        )

    def test_builder_selects_firecracker_default_snapshotter(self) -> None:
        reference = "docker.io/library/actrail-openeuler-workload:24.09"
        runner = ContainerdImageRunner(reference, present=True)
        requirements = KataRequirementsBuilder(
            backend="firecracker",
            runtime="io.containerd.kata.v2",
            runtime_config=Path("/etc/kata/configuration-fc.toml"),
            image=reference,
            runner=runner,
            pull_policy=PullPolicy.NEVER,
            image_archive=None,
            runtime_timeout_seconds=30,
            uid=1000,
            gid=39000,
            ready_timeout_seconds=10,
        ).build(
            name_prefix="firecracker",
            command=("/bin/true",),
            mounts=(),
            artifact_directories=(),
            labels=(),
            running_validator=lambda _: RequirementCheck.ready_check(),
        )
        spec = requirements.create_spec(
            ResolvedImage(reference, "sha256:abcd")
        )

        self.assertEqual(
            (
                requirements.profile.snapshotter,
                requirements.image.snapshotter,
                spec.snapshotter,
            ),
            ("devmapper", "devmapper", "devmapper"),
        )


if __name__ == "__main__":
    unittest.main()
