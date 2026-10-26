from __future__ import annotations

import sqlite3
import tempfile
import unittest
from pathlib import Path

from tests.v2.common.execution_isolation.evidence_database import (
    SandboxEvidenceDatabase,
)


class SandboxEvidenceDatabaseTest(unittest.TestCase):
    def test_counts_records_from_current_schema(self) -> None:
        with tempfile.TemporaryDirectory() as raw_directory:
            database = Path(raw_directory) / "sandbox-evidence.sqlite"
            with sqlite3.connect(database) as connection:
                connection.executescript(
                    """
                    CREATE TABLE sandbox_schema_meta (
                        singleton INTEGER PRIMARY KEY,
                        schema_version INTEGER NOT NULL
                    );
                    INSERT INTO sandbox_schema_meta(singleton, schema_version)
                    VALUES (1, 3);
                    CREATE TABLE sandbox_evidence (id INTEGER PRIMARY KEY);
                    INSERT INTO sandbox_evidence DEFAULT VALUES;
                    """
                )

            self.assertEqual(SandboxEvidenceDatabase(database).record_count(), 1)


if __name__ == "__main__":
    unittest.main()
