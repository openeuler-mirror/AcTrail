from __future__ import annotations

import copy
import json
import os
import re
import tomllib
from dataclasses import dataclass
from datetime import date, datetime, time
from pathlib import Path
from typing import Any, Mapping

from .constants import DEFAULTS


@dataclass(frozen=True)
class ConfigPatch:
    values: dict[str, Any]

    def save(self, path: Path) -> Path:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(self._table(self.values, ()), encoding="utf-8")
        return path

    @classmethod
    def _table(cls, values: Mapping[str, Any], names: tuple[str, ...]) -> str:
        lines = []
        for name, value in values.items():
            if not isinstance(value, dict):
                lines.append(f"{cls._key(name)} = {cls._value(value)}")
        if names and (lines or not values):
            lines.insert(0, "[" + ".".join(cls._key(name) for name in names) + "]")
        sections = ["\n".join(lines)] if lines else []
        for name, value in values.items():
            if isinstance(value, dict):
                sections.append(cls._table(value, (*names, name)).rstrip())
        return "\n\n".join(sections) + "\n"

    @staticmethod
    def _key(name: str) -> str:
        return name if re.fullmatch(r"[A-Za-z0-9_-]+", name) else json.dumps(name, ensure_ascii=False)

    @classmethod
    def _value(cls, value: Any) -> str:
        if isinstance(value, str):
            return json.dumps(value, ensure_ascii=False)
        if isinstance(value, bool):
            return str(value).lower()
        if isinstance(value, (int, float)):
            return repr(value)
        if isinstance(value, (datetime, date, time)):
            return value.isoformat()
        if isinstance(value, list):
            return "[" + ", ".join(cls._value(item) for item in value) + "]"
        if isinstance(value, dict):
            return "{ " + ", ".join(
                f"{cls._key(key)} = {cls._value(item)}"
                for key, item in value.items()
            ) + " }"
        raise TypeError(f"unsupported TOML value type: {type(value).__name__}")


class PatchBuilder:
    """Build an ordered configuration patch from runtime values and TOML files."""

    def __init__(self) -> None:
        self._values: dict[str, Any] = {}

    def with_values(self, values: dict[str, Any]) -> PatchBuilder:
        self._merge(self._values, values)
        return self

    def apply(self, path: Path) -> PatchBuilder:
        with path.open("rb") as source:
            return self.with_values(tomllib.load(source))

    def with_work_dir(self, work_dir: Path) -> PatchBuilder:
        work_dir = work_dir.resolve()
        return self.with_values({
            "control": {
                "socket_path": str(work_dir / "run/control.sock"),
                "pid_file": str(work_dir / "run/actraild.pid"),
                "log_path": str(work_dir / "log/actraild.log"),
            },
            "storage": {"sqlite": {"path": str(work_dir / "data/actrail.sqlite")}},
            "export": {"snapshot": {"directory": str(work_dir / "data/export")}},
            "payload": {"tls": {"sync_event_socket_path": str(work_dir / "run/tls-sync.sock")}},
            "cluster": {
                "report": {
                    "spool_dir": str(work_dir / "data/cluster-spool"),
                    "state_path": str(work_dir / "data/cluster-report-state.sqlite"),
                },
                "center": {"root_dir": str(work_dir / "data/cluster")},
            },
            "plugins": {"discovery": {"directory": str(work_dir / "plugins")}},
        })

    def with_plugin_directory(self, directory: Path) -> PatchBuilder:
        return self.with_values({
            "plugins": {"discovery": {"directory": str(directory.resolve())}}
        })

    def with_runtime_timing(self, shutdown_wait_ms: int = DEFAULTS.shutdown_wait_ms) -> PatchBuilder:
        defaults = {
            "SHUTDOWN_WAIT_MS": shutdown_wait_ms,
            "SUPERVISION_POLL_INTERVAL_MS": DEFAULTS.supervision_poll_interval_ms,
            "FINALIZATION_SHUTDOWN_DRAIN_TIMEOUT_MS": DEFAULTS.finalization_shutdown_drain_timeout_ms,
            "POST_TRACE_BROKER_REPLY_TIMEOUT_MS": DEFAULTS.post_trace_broker_reply_timeout_ms,
            "POST_TRACE_SHUTDOWN_DRAIN_TIMEOUT_MS": DEFAULTS.post_trace_shutdown_drain_timeout_ms,
            "PLUGIN_ALERT_DRAIN_TIMEOUT_MS": DEFAULTS.plugin_alert_drain_timeout_ms,
        }
        timing = {}
        for name, default in defaults.items():
            variable = "ACTRAIL_V2_" + name
            timing[name] = int(os.environ.get(variable, str(default)))
            if timing[name] < 1:
                raise ValueError(f"{variable} must be positive")
        return self.with_values({
            "supervision": {
                "shutdown_wait_ms": timing["SHUTDOWN_WAIT_MS"],
                "poll_interval_ms": timing["SUPERVISION_POLL_INTERVAL_MS"],
            },
            "control": {"finalization": {
                "shutdown_drain_timeout_ms": timing["FINALIZATION_SHUTDOWN_DRAIN_TIMEOUT_MS"],
                "post_trace": {
                    "broker_reply_timeout_ms": timing["POST_TRACE_BROKER_REPLY_TIMEOUT_MS"],
                    "shutdown_drain_timeout_ms": timing["POST_TRACE_SHUTDOWN_DRAIN_TIMEOUT_MS"],
                },
            }},
            "plugins": {"alerts": {"drain_timeout_ms": timing["PLUGIN_ALERT_DRAIN_TIMEOUT_MS"]}},
        })

    def build(self) -> ConfigPatch:
        return ConfigPatch(copy.deepcopy(self._values))

    @classmethod
    def _merge(cls, destination: dict[str, Any], values: dict[str, Any]) -> None:
        for key, value in values.items():
            current = destination.get(key)
            if isinstance(current, dict) and isinstance(value, dict):
                cls._merge(current, value)
            else:
                destination[key] = copy.deepcopy(value)
