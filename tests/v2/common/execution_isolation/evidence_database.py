from __future__ import annotations

import sqlite3
from pathlib import Path


class SandboxEvidenceDatabase:
    """Read-only record counter for actraild's independent evidence store."""

    def __init__(self, path: Path) -> None:
        self._path = path.resolve()

    def record_count(self) -> int:
        if not self._path.is_file():
            raise AssertionError(
                f"sandbox evidence database does not exist: {self._path}"
            )
        connection = sqlite3.connect(
            f"file:{self._path}?mode=ro",
            uri=True,
            timeout=1,
        )
        try:
            row = connection.execute(
                "SELECT COUNT(*) FROM sandbox_evidence"
            ).fetchone()
        finally:
            connection.close()
        if row is None:
            raise AssertionError("sandbox evidence count query returned no row")
        return int(row[0])
