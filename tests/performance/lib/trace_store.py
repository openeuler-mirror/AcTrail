"""Trace-scoped SQLite reads for benchmark validation."""

from __future__ import annotations

import sqlite3
from pathlib import Path


def read_trace_diagnostics(operator_config: Path, trace_id: int) -> str:
    storage_path = Path(operator_value(operator_config, "storage_sqlite_path"))
    with sqlite3.connect(storage_path) as connection:
        rows = connection.execute(
            """
            SELECT diagnostic_id, severity, kind, message
            FROM diagnostics
            WHERE trace_id = ?1
            ORDER BY emitted_at ASC, diagnostic_id ASC
            """,
            (trace_id,),
        ).fetchall()
    lines = ["DIAG SEVERITY KIND MESSAGE"]
    for diagnostic_id, severity, kind, message in rows:
        lines.append(f"diag-{diagnostic_id} {severity} {kind} {message}")
    return "\n".join(lines)


def operator_value(operator_config: Path, key: str) -> str:
    section = ""
    for raw_line in operator_config.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            section = line.strip("[]")
            continue
        if "=" not in line:
            continue
        parsed_key, value = line.split("=", 1)
        if remap_operator_key(section, parsed_key.strip()) == key:
            return unquote(value.strip())
    raise RuntimeError(f"{key} is missing from {operator_config}")


def remap_operator_key(section: str, key: str) -> str:
    if section == "storage.sqlite" and key == "path":
        return "storage_sqlite_path"
    return key


def unquote(value: str) -> str:
    if len(value) >= 2 and value.startswith('"') and value.endswith('"'):
        return value[1:-1]
    return value
