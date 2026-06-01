"""Tracefs helpers."""

from __future__ import annotations

import re
from pathlib import Path


def validate_trace_group(group: str) -> None:
    if not re.match(r"^[A-Za-z_][A-Za-z0-9_]*$", group):
        raise RuntimeError(f"invalid trace event group: {group}")


def write_text(path: Path, text: str) -> None:
    with path.open("w", encoding="utf-8") as handle:
        handle.write(text)
