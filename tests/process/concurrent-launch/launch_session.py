"""Incremental output collection for one concurrent actrailctl launch."""

from __future__ import annotations

import os
import re
import select
import subprocess
import sys
import time


TRACE_RE = re.compile(r"trace trace-(\d+) entered Active")


class LaunchSession:
    """Own one launch process and its incrementally collected output."""

    def __init__(self, process: subprocess.Popen[bytes]) -> None:
        self.process = process
        self.stdout = bytearray()
        self.stderr = bytearray()
        self.printed_stdout_bytes = 0

    def wait_for_trace(self, timeout_sec: float) -> int:
        deadline = time.monotonic() + timeout_sec
        while time.monotonic() < deadline:
            self._read_ready(deadline)
            trace_id = self._trace_id()
            if trace_id is not None:
                self._print_new_stdout()
                return trace_id
            if self.process.poll() is not None:
                self._collect_completed()
                trace_id = self._trace_id()
                if trace_id is not None:
                    self._print_new_stdout()
                    return trace_id
                raise RuntimeError(
                    "launch exited before trace id "
                    f"exit={self.process.returncode} "
                    f"stdout={self._stdout_text()} stderr={self._stderr_text()}"
                )
        raise RuntimeError(
            f"timed out waiting for trace id stdout={self._stdout_text()} "
            f"stderr={self._stderr_text()}"
        )

    def wait_for_completion(self, timeout_sec: float) -> str:
        remaining_stdout, remaining_stderr = self.process.communicate(timeout=timeout_sec)
        self.stdout.extend(remaining_stdout)
        self.stderr.extend(remaining_stderr)
        self._print_new_stdout()
        stderr = self._stderr_text()
        if stderr:
            print(stderr, end="", file=sys.stderr)
        if self.process.returncode != 0:
            raise RuntimeError(
                f"launch failed exit={self.process.returncode}\n"
                f"stdout={self._stdout_text()}\nstderr={stderr}"
            )
        return self._stdout_text()

    def terminate(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()

    def _read_ready(self, deadline: float) -> None:
        assert self.process.stdout is not None
        assert self.process.stderr is not None
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return
        readable, _, _ = select.select(
            [self.process.stdout, self.process.stderr], [], [], remaining
        )
        for stream in readable:
            chunk = os.read(stream.fileno(), 65536)
            if stream is self.process.stdout:
                self.stdout.extend(chunk)
            else:
                self.stderr.extend(chunk)

    def _collect_completed(self) -> None:
        remaining_stdout, remaining_stderr = self.process.communicate()
        self.stdout.extend(remaining_stdout)
        self.stderr.extend(remaining_stderr)

    def _trace_id(self) -> int | None:
        match = TRACE_RE.search(self._stdout_text())
        return int(match.group(1)) if match else None

    def _print_new_stdout(self) -> None:
        if self.printed_stdout_bytes == len(self.stdout):
            return
        print(self.stdout[self.printed_stdout_bytes :].decode("utf-8"), end="")
        self.printed_stdout_bytes = len(self.stdout)

    def _stdout_text(self) -> str:
        return self.stdout.decode("utf-8")

    def _stderr_text(self) -> str:
        return self.stderr.decode("utf-8")
