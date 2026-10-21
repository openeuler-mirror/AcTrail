from __future__ import annotations

import hashlib
from pathlib import Path

from .constants import DEFAULTS


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(DEFAULTS.checksum_read_chunk_bytes), b""):
                digest.update(chunk)
    except OSError as error:
        raise RuntimeError(f"cannot read artifact file {path}: {error}") from error
    return digest.hexdigest()
