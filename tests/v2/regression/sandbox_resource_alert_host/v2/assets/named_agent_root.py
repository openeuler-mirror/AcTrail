#!/usr/bin/env python3
from __future__ import annotations

import ctypes
import os
import subprocess
import sys
import time
from pathlib import Path


class NamedAgentRoot:
    _PR_SET_NAME = 15
    _PROCESS_NAME = b"actrail-root"

    def __init__(self) -> None:
        self._agent = self._required_path("ACTRAIL_HOST_REAL_XIAOO")
        self._pid_file = self._required_path("ACTRAIL_HOST_ROOT_PID_FILE")
        self._release = self._required_path("ACTRAIL_HOST_CHILD_RELEASE")
        self._timeout_seconds = self._positive_timeout()

    def run(self, arguments: list[str]) -> int:
        self._set_process_name()
        self._pid_file.write_text(f"{os.getpid()}\n", encoding="ascii")
        self._wait_for_release()
        return subprocess.run((str(self._agent), *arguments), check=False).returncode

    def _set_process_name(self) -> None:
        libc = ctypes.CDLL(None, use_errno=True)
        if libc.prctl(self._PR_SET_NAME, self._PROCESS_NAME, 0, 0, 0) != 0:
            error = ctypes.get_errno()
            raise OSError(error, os.strerror(error))

    def _wait_for_release(self) -> None:
        deadline = time.monotonic() + self._timeout_seconds
        while time.monotonic() < deadline:
            if self._release.is_file():
                return
            time.sleep(0.05)
        raise RuntimeError(f"timed out waiting for child release: {self._release}")

    @staticmethod
    def _required_path(name: str) -> Path:
        value = os.environ.get(name)
        if not value:
            raise RuntimeError(f"{name} is required")
        return Path(value)

    @staticmethod
    def _positive_timeout() -> int:
        raw = os.environ.get("ACTRAIL_HOST_CHILD_TIMEOUT_SECONDS", "90")
        value = int(raw)
        if value <= 0:
            raise RuntimeError("ACTRAIL_HOST_CHILD_TIMEOUT_SECONDS must be positive")
        return value


if __name__ == "__main__":
    raise SystemExit(NamedAgentRoot().run(sys.argv[1:]))
