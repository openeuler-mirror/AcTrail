from __future__ import annotations

import subprocess
from dataclasses import dataclass, field
from pathlib import Path


@dataclass(frozen=True)
class ContainerImage:
    """Ensures a content-addressed test image exists without rebuilding it."""

    image_name: str
    version: str = "latest"
    dockerfile_path: Path | None = None
    build_context: Path | None = None
    build_args: dict[str, str] = field(default_factory=dict)
    force_rebuild: bool = False

    @property
    def reference(self) -> str:
        return f"{self.image_name}:{self.version}"

    def ensure(self) -> str:
        if not self.force_rebuild and self._exists():
            return self.reference
        if self.dockerfile_path is None:
            self._run(["docker", "pull", self.reference], "pull image")
            return self.reference
        if self.build_context is None:
            raise ValueError("build_context is required when dockerfile_path is set")

        dockerfile = self.dockerfile_path.resolve()
        context = self.build_context.resolve()
        if not dockerfile.is_file():
            raise FileNotFoundError(f"Dockerfile does not exist: {dockerfile}")
        if not context.is_dir():
            raise NotADirectoryError(f"build context does not exist: {context}")

        command = ["docker", "build", "-q", "-f", str(dockerfile)]
        for name, value in sorted(self.build_args.items()):
            command.extend(["--build-arg", f"{name}={value}"])
        command.extend(["-t", self.reference, str(context)])
        self._run(command, "build image")
        return self.reference

    def _exists(self) -> bool:
        result = subprocess.run(
            ["docker", "image", "inspect", self.reference],
            text=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode == 0:
            return True
        if "No such image" in result.stderr:
            return False
        raise RuntimeError(
            f"failed to inspect image {self.reference}: {result.stderr.strip()}"
        )

    @staticmethod
    def _run(command: list[str], operation: str) -> None:
        result = subprocess.run(
            command,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode != 0:
            diagnostic = (result.stderr or result.stdout).strip()
            raise RuntimeError(
                f"failed to {operation} exit={result.returncode}: {diagnostic}"
            )
