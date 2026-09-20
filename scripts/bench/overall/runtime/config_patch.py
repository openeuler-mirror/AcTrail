"""Merge operator settings while retaining benchmark isolation."""

from __future__ import annotations

import json
from pathlib import Path
import tomllib


class ConfigPatch:
    def __init__(self, path: Path):
        self.path = path.resolve()
        try:
            self.values = tomllib.loads(self.path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            raise ValueError(f"cannot load TOML config patch: {self.path}") from None

    def apply_isolation(self, isolated_patch: Path) -> None:
        isolation = tomllib.loads(isolated_patch.read_text(encoding="utf-8"))
        self._merge(self.values, isolation)
        rendered = "\n".join(
            f"{json.dumps(key)} = {self._render(value)}"
            for key, value in self.values.items()
        )
        isolated_patch.write_text(rendered + "\n", encoding="utf-8")

    @classmethod
    def _merge(cls, base: dict, overrides: dict) -> None:
        for key, value in overrides.items():
            if isinstance(value, dict) and isinstance(base.get(key), dict):
                cls._merge(base[key], value)
            else:
                base[key] = value

    @classmethod
    def _render(cls, value: object) -> str:
        if isinstance(value, dict):
            fields = (
                f"{json.dumps(key)} = {cls._render(item)}"
                for key, item in value.items()
            )
            return "{ " + ", ".join(fields) + " }"
        if isinstance(value, list):
            return "[" + ", ".join(cls._render(item) for item in value) + "]"
        if isinstance(value, (str, bool, int, float)):
            return json.dumps(value, ensure_ascii=False, allow_nan=False)
        raise ValueError("config patch contains a value unsupported by operator settings")
