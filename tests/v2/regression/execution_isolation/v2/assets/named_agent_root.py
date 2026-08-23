#!/usr/bin/env python3
from __future__ import annotations

import ctypes
import os
import subprocess
import sys
import time
from pathlib import Path


_PR_SET_NAME = 15
_ROOT_COMM = b"actrail-root"


class NamedAgentRoot:
    def __init__(self) -> None:
        self._agent = self._required_path("ACTRAIL_EXECUTION_REAL_XIAOO")
        self._pid_file = self._required_path("ACTRAIL_EXECUTION_ROOT_PID_FILE")
        self._child_release = self._required_path(
            "ACTRAIL_EXECUTION_CHILD_RELEASE"
        )
        self._timeout_seconds = self._positive_timeout()

    def run(self, arguments: list[str]) -> int:
        self._set_process_name()
        self._pid_file.write_text(f"{os.getpid()}\n", encoding="ascii")
        self._wait_for_child_release()
        result = subprocess.run((str(self._agent), *arguments), check=False)
        return result.returncode

    def _set_process_name(self) -> None:
        libc = ctypes.CDLL(None, use_errno=True)
        if libc.prctl(_PR_SET_NAME, _ROOT_COMM, 0, 0, 0) != 0:
            error = ctypes.get_errno()
            raise OSError(error, os.strerror(error))

    def _wait_for_child_release(self) -> None:
        deadline = time.monotonic() + self._timeout_seconds
        while time.monotonic() < deadline:
            if self._child_release.is_file():
                return
            time.sleep(0.05)
        raise RuntimeError(
            f"timed out waiting for child release: {self._child_release}"
        )

    @staticmethod
    def _required_path(name: str) -> Path:
        value = os.environ.get(name)
        if not value:
            raise RuntimeError(f"{name} is required")
        return Path(value)

    @staticmethod
    def _positive_timeout() -> int:
        raw = os.environ.get("ACTRAIL_EXECUTION_CHILD_TIMEOUT_SECONDS", "90")
        try:
            value = int(raw)
        except ValueError as error:
            raise RuntimeError(
                "ACTRAIL_EXECUTION_CHILD_TIMEOUT_SECONDS must be an integer"
            ) from error
        if value <= 0:
            raise RuntimeError(
                "ACTRAIL_EXECUTION_CHILD_TIMEOUT_SECONDS must be positive"
            )
        return value


if __name__ == "__main__":
    raise SystemExit(NamedAgentRoot().run(sys.argv[1:]))
