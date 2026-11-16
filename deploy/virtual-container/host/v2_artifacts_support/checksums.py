"""SHA-256 of artifact files."""

from __future__ import annotations

import hashlib
from pathlib import Path


_READ_CHUNK_BYTES = 1024 * 1024


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(_READ_CHUNK_BYTES), b""):
                digest.update(chunk)
    except OSError as error:
        raise RuntimeError(f"cannot read artifact file {path}: {error}") from error
    return digest.hexdigest()
