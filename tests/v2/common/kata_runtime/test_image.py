from __future__ import annotations

import tempfile
import unittest
from collections.abc import Sequence
from pathlib import Path

from tests.v2.common.kata_runtime.image import ContainerdImage, PullPolicy
from tests.v2.common.kata_runtime.process import CommandResult
from tests.v2.common.kata_runtime.requirements import PreparePolicy


class ContainerdImageRunner:
    def __init__(self, reference: str, *, present: bool) -> None:
        self.reference = reference
        self.present = present
        self.commands: list[tuple[str, ...]] = []

    def run(
        self,
        argv: Sequence[str],
        *,
        timeout: float | None = None,
    ) -> CommandResult:
        command = tuple(argv)
        self.commands.append(command)
        del timeout
        if command[3:6] == ("images", "list", "--quiet"):
            output = f"{self.reference}\n" if self.present else ""
            return CommandResult(command, 0, output, "")
        if command[3:5] == ("images", "list"):
            output = (
                "REF TYPE DIGEST SIZE PLATFORMS LABELS\n"
                f"{self.reference} application/vnd.oci.image.manifest.v1+json "
                "sha256:abcd 233.9MiB linux/arm64 -\n"
            )
            return CommandResult(command, 0, output, "")
        if command[3:5] == ("images", "import"):
            self.present = True
            return CommandResult(command, 0, "imported\n", "")
        return CommandResult(command, 1, "", f"unexpected command: {command}")


class ContainerdImageTest(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
