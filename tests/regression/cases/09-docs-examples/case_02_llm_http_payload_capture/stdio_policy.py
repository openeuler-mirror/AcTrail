"""Stdio retention policy helpers for docs HTTP payload regression."""

from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path

CONFIG_FALSE = "false"
CONFIG_TRUE = "true"
STORAGE_MODE_DROP = "drop"


def curl_stdout_payload_is_stored(config: Mapping[str, str]) -> bool:
    """Return whether successful curl response stdout should create a Stdio row."""
    if config_bool(config, "payload_stdio_enabled") is False:
        return False
    if config_bool(config, "payload_stdio_capture_stdout") is False:
        return False
    return required_value(config, "payload_stdio_stdout_storage_mode") != STORAGE_MODE_DROP


def curl_stdout_payload_is_stored_for_operator_config(path: Path) -> bool:
    return curl_stdout_payload_is_stored(read_top_level_key_values(path))


def read_top_level_key_values(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    section = ""
    array_depth = 0
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if array_depth:
            array_depth += line.count("[") - line.count("]")
            if array_depth < 0:
                raise RuntimeError(f"invalid operator config array in {path}: {raw_line!r}")
            continue
        if line.startswith("[") and line.endswith("]"):
            section = line.strip("[]")
            continue
        key, separator, value = line.partition("=")
        if separator != "=":
            raise RuntimeError(f"invalid operator config line: {raw_line!r}")
        array_depth = value.count("[") - value.count("]")
        if array_depth < 0:
            raise RuntimeError(f"invalid operator config array in {path}: {raw_line!r}")
        remapped = operator_config_key(section, key.strip())
        if remapped is not None:
            values[remapped] = unquote(value.strip())
    if array_depth:
        raise RuntimeError(f"unterminated operator config array in {path}")
    return values


def operator_config_key(section: str, key: str) -> str | None:
    if section == "payload.stdio":
        return f"payload_stdio_{key}"
    if not section:
        return key
    return None


def unquote(value: str) -> str:
    if len(value) >= 2 and value.startswith('"') and value.endswith('"'):
        return value[1:-1]
    return value


def config_bool(config: Mapping[str, str], key: str) -> bool:
    value = required_value(config, key)
    if value == CONFIG_TRUE:
        return True
    if value == CONFIG_FALSE:
        return False
    raise RuntimeError(f"invalid boolean config {key}={value!r}")


def required_value(config: Mapping[str, str], key: str) -> str:
    value = config.get(key)
    if value is None or value == "":
        raise RuntimeError(f"missing required config key {key}")
    return value
