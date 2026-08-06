from __future__ import annotations

import ast
import re
from pathlib import Path
from typing import Any

from .backend import kata_backend

try:
    import tomllib
except ModuleNotFoundError:
    tomllib = None


def load_hypervisor_table(config: Path, backend: str) -> dict[str, Any]:
    """Load the selected Kata hypervisor table on Python 3.10 and newer."""

    section = kata_backend(backend).toml_section
    table_name = section.removeprefix("hypervisor.")
    if tomllib is not None:
        with config.open("rb") as source:
            document = tomllib.load(source)
        table = document["hypervisor"][table_name]
        if not isinstance(table, dict):
            raise ValueError(f"[{section}] is not a table")
        return table

    assignments: dict[str, Any] = {}
    active = False
    pending = ""
    with config.open("r", encoding="utf-8") as source:
        for raw_line in source:
            line = raw_line.strip()
            if not line or line.startswith("#"):
                continue
            if line.startswith("[") and line.endswith("]"):
                if active:
                    break
                active = line == f"[{section}]"
                continue
            if not active:
                continue
            pending = f"{pending} {line}".strip()
            if pending.count("[") > pending.count("]"):
                continue
            match = re.match(r"^([A-Za-z0-9_]+)\s*=\s*(.+)$", pending)
            pending = ""
            if match is None:
                continue
            key, literal = match.groups()
            literal = re.sub(r"\s+#.*$", "", literal)
            try:
                assignments[key] = ast.literal_eval(literal)
            except (SyntaxError, ValueError):
                continue
    if not assignments:
        raise KeyError(section)
    return assignments


def runtime_path(config: Path, backend: str, key: str) -> Path:
    table = load_hypervisor_table(config, backend)
    value = table.get(key)
    if not isinstance(value, str) or not value:
        raise ValueError(
            f"runtime config {config} does not define "
            f"[{kata_backend(backend).toml_section}].{key}"
        )
    return Path(value)
