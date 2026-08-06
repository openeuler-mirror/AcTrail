from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from pathlib import Path

from .process import CommandResult, CommandRunner
from .requirements import PreparePolicy, ResolvedImage


class PullPolicy(str, Enum):
    NEVER = "never"
    MISSING = "missing"
    ALWAYS = "always"


@dataclass(frozen=True)
class ContainerdImage:
    """Resolves one containerd image and lazily prepares it when allowed."""

    reference: str
    runner: CommandRunner
    namespace: str = "default"
    pull_policy: PullPolicy = PullPolicy.NEVER
    archive: Path | None = None
    prepare_command: tuple[str, ...] | None = None
    timeout_seconds: float = 600

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
        if self.prepare_command is not None and (
            not self.prepare_command or any(not value for value in self.prepare_command)
        ):
            raise ValueError("image prepare command must contain non-empty argv")
        if self.timeout_seconds <= 0:
            raise ValueError("image preparation timeout must be positive")

    def ensure(self, policy: PreparePolicy) -> ResolvedImage:
        present = self._exists()
        if policy is PreparePolicy.CHECK_ONLY:
            if not present:
                raise RuntimeError(
                    f"required containerd image is missing: {self.reference}"
                )
            return self._resolve()

        if self.pull_policy is PullPolicy.NEVER:
            if not present:
                raise RuntimeError(
                    "required containerd image is missing and pull policy is never: "
                    + self.reference
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
            self._run_checked(
                self._ctr("images", "import", str(self.archive)),
                "import containerd image archive",
            )
            return
        self._run_checked(
            self._ctr("images", "pull", self.reference),
            "pull containerd image",
        )

    def _exists(self) -> bool:
        result = self.runner.run(
            self._ctr(
                "images",
                "list",
                "--quiet",
                f"name=={self.reference}",
            ),
            timeout=self.timeout_seconds,
        )
        if result.returncode != 0:
            raise _image_command_error("list containerd images", result)
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
