from __future__ import annotations

import os
from pathlib import Path


def positive_environment_int(name: str, default: str) -> int:
    return bounded_environment_int(
        name,
        default,
        minimum=1,
        maximum=2147483647,
    )


def bounded_environment_int(
    name: str,
    default: str,
    *,
    minimum: int,
    maximum: int,
) -> int:
    raw = os.environ.get(name, default)
    try:
        value = int(raw)
    except ValueError as error:
        raise ValueError(f"{name} must be an integer") from error
    if value < minimum or value > maximum:
        raise ValueError(f"{name} must be between {minimum} and {maximum}")
    return value


def optional_absolute_path(value: str | None, name: str) -> Path | None:
    return absolute_path(value, name) if value else None


def absolute_path(value: str, name: str) -> Path:
    path = Path(value).expanduser()
    if not path.is_absolute():
        raise ValueError(f"{name} must be an absolute path: {path}")
    return path
