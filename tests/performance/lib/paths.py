"""Path helpers for performance benchmark scripts."""

from __future__ import annotations

import os
import shutil
from pathlib import Path


def resolve(repo: Path, raw: str) -> Path:
    path = Path(raw)
    if path.is_absolute():
        return path
    return repo / path


def resolve_command(repo: Path, raw: str) -> Path:
    path = Path(raw)
    if path.is_absolute() or len(path.parts) > 1:
        return resolve(repo, raw)
    found = shutil.which(raw)
    if not found:
        raise RuntimeError(f"command not found on PATH: {raw}")
    return Path(found)


def require_file(path: Path, executable: bool) -> None:
    if not path.exists():
        raise RuntimeError(f"missing required file {path}")
    if executable and not os.access(path, os.X_OK):
        raise RuntimeError(f"required file is not executable: {path}")
