from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class ProcessDefaults:
    terminate_grace_seconds: float = 2.0


@dataclass(frozen=True)
class WebDefaults:
    plugin_web_stop_timeout_seconds: float = 10.0
    web_command_timeout_margin_seconds: float = 1.0


@dataclass(frozen=True)
class DatabaseDefaults:
    database_timeout_seconds: float = 1.0


@dataclass(frozen=True)
class RuntimeDefaults:
    clean_control_state: bool = True
    echo_commands: bool = True


PROCESS_DEFAULTS = ProcessDefaults()
WEB_DEFAULTS = WebDefaults()
DATABASE_DEFAULTS = DatabaseDefaults()
RUNTIME_DEFAULTS = RuntimeDefaults()
