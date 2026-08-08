"""Best-effort host environment summary printed at regression startup."""

from __future__ import annotations

import os
import platform
import socket
from pathlib import Path

from ..core import TestOutput


class EnvironmentProbe:
    """Collect and print host characteristics once per regression run.

    Collection is best-effort: a field that cannot be read is skipped
    instead of failing the run, keeping probe failures local to this step.
    """

    _OS_RELEASE_CANDIDATES = (
        Path("/etc/os-release"),
        Path("/usr/lib/os-release"),
    )
    _FIELD_ORDER = (
        "kernel_release",
        "os",
        "machine",
        "hostname",
        "python",
    )

    def __init__(self, output: TestOutput) -> None:
        self._output = output

    def print_summary(self) -> None:
        fields = self._collect()
        self._output.heading("▶ environment")
        for name in self._FIELD_ORDER:
            value = fields.get(name)
            if value is not None:
                self._output.line(f"{name}={value}")

    def _collect(self) -> dict[str, str]:
        fields: dict[str, str] = {}
        uname = os.uname()
        fields["kernel_release"] = uname.release
        fields["machine"] = uname.machine
        fields["hostname"] = socket.gethostname()
        fields["python"] = platform.python_version()
        os_pretty_name = self._os_pretty_name()
        if os_pretty_name is not None:
            fields["os"] = os_pretty_name
        return fields

    def _os_pretty_name(self) -> str | None:
        for path in self._OS_RELEASE_CANDIDATES:
            pretty_name = self._read_pretty_name(path)
            if pretty_name is not None:
                return pretty_name
        return None

    @staticmethod
    def _read_pretty_name(path: Path) -> str | None:
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except OSError:
            return None
        for line in lines:
            if not line.startswith("PRETTY_NAME="):
                continue
            value = line.split("=", 1)[1].strip()
            if len(value) >= 2 and value[0] == value[-1] == '"':
                value = value[1:-1]
            return value or None
        return None
