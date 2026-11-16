"""Name normalization shared by tool alias matching."""

from __future__ import annotations


def normalize_name(value: str) -> str:
    """Mechanical key normalization: casefold and drop separators.

    ``filePath``, ``file_path`` and ``filepath`` all normalize to
    ``filepath``, so camelCase/snake_case variants need no explicit aliases.
    """

    return "".join(ch for ch in value.casefold() if ch.isalnum())
