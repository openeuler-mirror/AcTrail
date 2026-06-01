"""Simple key/value workload config parsing for regression cases."""

from __future__ import annotations

from pathlib import Path


def read_config(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        key, value = line.split("=", 1)
        values[key.strip()] = value.strip()
    return values


def required(values: dict[str, str], key: str) -> str:
    value = values.get(key)
    if value is None or value == "":
        raise RuntimeError(f"missing required config key {key}")
    return value
