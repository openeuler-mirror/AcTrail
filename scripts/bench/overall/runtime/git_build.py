"""Release build and git commit verification for the overall benchmark."""

from __future__ import annotations

import subprocess
from pathlib import Path
from typing import Sequence


class ReleaseBuild:
    """Build the release binaries and verify the checked-out commit.

    The commit id is captured before and after ``cargo build --release`` so a
    build that pulls in or produces commits cannot silently skew the result.
    """

    def __init__(self, repo_root: Path):
        self._repo_root = repo_root

    def ensure(self, *, timeout_seconds: float = 3600.0) -> dict[str, str]:
        before = self.commit_info()
        self._cargo_build(timeout_seconds)
        after = self.commit_info()
        if after["id"] != before["id"]:
            raise RuntimeError(
                f"commit id changed during cargo build: "
                f"{before['id']} -> {after['id']}"
            )
        return after

    def commit_info(self) -> dict[str, str]:
        commit_id = self._git(["rev-parse", "HEAD"]).strip()
        title = self._git(["log", "-1", "--format=%s"]).strip()
        return {"id": commit_id, "title": title}

    def _cargo_build(self, timeout_seconds: float) -> None:
        completed = subprocess.run(
            ["cargo", "build", "--release"],
            cwd=str(self._repo_root),
            capture_output=True,
            text=True,
            timeout=timeout_seconds,
            check=False,
        )
        if completed.returncode != 0:
            detail = (
                completed.stderr.strip() or completed.stdout.strip()
            )[-3000:]
            raise RuntimeError(
                "cargo build --release failed "
                f"({completed.returncode})\n{detail}"
            )

    def _git(self, arguments: Sequence[str]) -> str:
        completed = subprocess.run(
            ["git", "-C", str(self._repo_root), *arguments],
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )
        if completed.returncode != 0:
            detail = completed.stderr.strip()[-1000:]
            raise RuntimeError(
                f"git {' '.join(arguments)} failed: {detail}"
            )
        return completed.stdout
