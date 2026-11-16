"""Stop a test process group and wait for its leader to exit."""

from __future__ import annotations

import os
import signal
import subprocess


class ProcessGroupTerminator:
    def __init__(self, grace_seconds: float):
        self._grace_seconds = grace_seconds

    def terminate(self, process: subprocess.Popen[str]) -> None:
        if process.poll() is not None:
            return
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=self._grace_seconds)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
