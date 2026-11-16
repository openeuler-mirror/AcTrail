from __future__ import annotations

import re
from dataclasses import dataclass
from enum import Enum
from pathlib import Path

from v2_artifacts_support.containerd_image import (
    containerd_image_check_ready,
    firecracker_workload_reference,
    verified_image_archive_snapshot,
)

from .process import CommandResult, CommandRunner
from .requirements import PreparePolicy, ResolvedImage

from .constants import DEFAULTS


_SHA256 = re.compile(r"^[0-9a-f]{64}$")


class PullPolicy(str, Enum):
    NEVER = "never"
    MISSING = "missing"
    ALWAYS = "always"


@dataclass(frozen=True)
class ContainerdImage:
    """Resolves one containerd image and lazily prepares it when allowed."""

    reference: str
    runner: CommandRunner
    namespace: str = DEFAULTS.namespace
    pull_policy: PullPolicy = PullPolicy.NEVER
    archive: Path | None = None
    archive_sha256: str | None = None
    prepare_command: tuple[str, ...] | None = None
    timeout_seconds: float = DEFAULTS.image_timeout_seconds
    snapshotter: str | None = None

    def __post_init__(self) -> None:
        if not self.reference:
            raise ValueError("containerd image reference must not be empty")
        if not self.namespace:
            raise ValueError("containerd namespace must not be empty")
        if not isinstance(self.pull_policy, PullPolicy):
            raise ValueError(f"unsupported image pull policy: {self.pull_policy}")
        if self.archive is not None and not self.archive.is_absolute():
            raise ValueError(
                f"containerd image archive must be absolute: {self.archive}"
            )
        if self.archive_sha256 is not None:
            if self.archive is None:
                raise ValueError(
                    "containerd image archive digest requires an archive"
                )
            if _SHA256.fullmatch(self.archive_sha256) is None:
                raise ValueError(
                    "containerd image archive digest must be 64 lowercase "
                    "hexadecimal characters"
                )
        if self.prepare_command is not None and (
            not self.prepare_command or any(not value for value in self.prepare_command)
        ):
            raise ValueError("image prepare command must contain non-empty argv")
        if self.timeout_seconds <= 0:
            raise ValueError("image preparation timeout must be positive")
        if self.snapshotter is not None and not self.snapshotter.strip():
            raise ValueError("containerd snapshotter must not be empty")

    def ensure(self, policy: PreparePolicy) -> ResolvedImage:
        present = self._exists()
        if policy is PreparePolicy.CHECK_ONLY:
            if not present:
                raise RuntimeError(self._missing_image_message())
            return self._resolve()

        if self.pull_policy is PullPolicy.NEVER:
            if not present:
                raise RuntimeError(
                    self._missing_image_message()
                    + "; pull policy is never"
                )
            return self._resolve()

        if present and self.pull_policy is not PullPolicy.ALWAYS:
            return self._resolve()

        self._prepare()
        if not self._exists():
            raise RuntimeError(
                "image preparation completed without registering the requested "
                f"containerd image: {self.reference}"
            )
        return self._resolve()

    def refresh(self, reason: str) -> ResolvedImage:
        if not reason:
            raise ValueError("image refresh requires a diagnostic reason")
        if self.pull_policy is PullPolicy.NEVER:
            raise RuntimeError(
                "containerd image cannot refresh with pull policy never: "
                + self.reference
            )
        self._prepare()
        if not self._exists():
            raise RuntimeError(
                "image refresh completed without registering the requested "
                f"containerd image: {self.reference}"
            )
        return self._resolve()

    def _prepare(self) -> None:
        if self.prepare_command is not None:
            self._run_checked(self.prepare_command, "prepare containerd image")
            return
        if self.archive is not None:
            if not self.archive.is_file():
                raise FileNotFoundError(
                    f"containerd image archive does not exist: {self.archive}"
                )
            if self.archive_sha256 is not None:
                with verified_image_archive_snapshot(
                    self.archive,
                    self.archive_sha256,
                ) as snapshot:
                    self._import_archive(snapshot)
                return
            self._import_archive(self.archive)
            return
        self._run_checked(
            self._ctr(
                "images",
                "pull",
                *self._snapshotter_option(),
                self.reference,
            ),
            "pull containerd image",
        )

    def _import_archive(self, archive: Path) -> None:
        self._run_checked(
            self._ctr(
                "images",
                "import",
                *self._snapshotter_option(),
                str(archive),
            ),
            "import containerd image archive",
        )

    def _exists(self) -> bool:
        if self.snapshotter is not None:
            command = self._ctr(
                "images",
                "check",
                *self._snapshotter_option(),
                f"name=={self.reference}",
            )
            operation = "check containerd image snapshot"
        else:
            command = self._ctr(
                "images",
                "list",
                "--quiet",
                f"name=={self.reference}",
            )
            operation = "list containerd images"
        result = self.runner.run(
            command,
            timeout=self.timeout_seconds,
        )
        if result.returncode != 0:
            raise _image_command_error(operation, result)
        if self.snapshotter is not None:
            return containerd_image_check_ready(result.stdout, self.reference)
        return self.reference in {
            line.strip() for line in result.stdout.splitlines() if line.strip()
        }

    def _resolve(self) -> ResolvedImage:
        result = self.runner.run(
            self._ctr("images", "list", f"name=={self.reference}"),
            timeout=self.timeout_seconds,
        )
        if result.returncode != 0:
            raise _image_command_error("inspect containerd image", result)
        for line in result.stdout.splitlines()[1:]:
            columns = line.split()
            if len(columns) >= 3 and columns[0] == self.reference:
                digest = columns[2] if columns[2].startswith("sha256:") else None
                return ResolvedImage(self.reference, digest)
        raise RuntimeError(
            f"containerd image disappeared while resolving it: {self.reference}"
        )

    def _ctr(self, *arguments: str) -> list[str]:
        return ["ctr", "-n", self.namespace, *arguments]

    def _snapshotter_option(self) -> tuple[str, ...]:
        if self.snapshotter is None:
            return ()
        return ("--snapshotter", self.snapshotter)

    def _missing_image_message(self) -> str:
        if self.snapshotter is None:
            return f"required containerd image is missing: {self.reference}"
        return (
            "required containerd image is missing, incomplete, or not unpacked "
            f"for snapshotter {self.snapshotter}: {self.reference}"
        )

    def _run_checked(
        self,
        command: tuple[str, ...] | list[str],
        operation: str,
    ) -> None:
        result = self.runner.run(command, timeout=self.timeout_seconds)
        if result.returncode != 0:
            raise _image_command_error(operation, result)


def _image_command_error(operation: str, result: CommandResult) -> RuntimeError:
    return RuntimeError(
        f"failed to {operation} exit={result.returncode}: "
        f"{result.diagnostic or 'no diagnostic output'}"
    )
