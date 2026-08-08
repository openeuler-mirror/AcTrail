from __future__ import annotations

import select
import subprocess
import sys
import time
from pathlib import Path
from typing import TextIO


class LocalXiaooProvider:
    """Deterministic streaming provider that asks real Xiaoo for one Bash call."""

    def __init__(
        self,
        repo: Path,
        work_dir: Path,
        marker: Path,
        ready_timeout_seconds: float,
    ):
        self._proxy = repo / "tests/support/llm-http-proxy/provider_proxy.py"
        self._stderr_path = work_dir / "provider.stderr.log"
        self._marker = marker
        self._ready_timeout_seconds = ready_timeout_seconds
        self._process: subprocess.Popen[str] | None = None
        self._stderr: TextIO | None = None
        self.base_url: str | None = None

    def start(self) -> str:
        if not self._proxy.is_file():
            raise RuntimeError(f"local provider proxy is missing: {self._proxy}")
        self._stderr = self._stderr_path.open("w", encoding="utf-8")
        self._process = subprocess.Popen(
            [
                sys.executable,
                str(self._proxy),
                "--mode",
                "local-stream",
                "--bind-host",
                "127.0.0.1",
                "--bind-port",
                "0",
                "--local-stream-response-text",
                "ACTRAIL_XIAOO_COMMAND_POLICY_OK",
                "--local-stream-reasoning-tokens",
                "1",
                "--local-tool-command",
                f"printf ACTRAIL_XIAOO_COMMAND_OK > {self._marker}",
            ],
            text=True,
            stdout=subprocess.PIPE,
            stderr=self._stderr,
        )
        deadline = time.monotonic() + self._ready_timeout_seconds
        if self._process.stdout is None:
            raise RuntimeError("local provider stdout is unavailable")
        while time.monotonic() < deadline:
            readable, _, _ = select.select(
                [self._process.stdout], [], [], max(0.0, deadline - time.monotonic())
            )
            if readable:
                line = self._process.stdout.readline()
                print(line, end="")
                if line.startswith("proxy_base_url="):
                    self.base_url = line.split("=", 1)[1].strip()
                    return self.base_url
            if self._process.poll() is not None:
                raise RuntimeError(
                    "local provider exited before ready: " + self._read_stderr()
                )
        raise RuntimeError("local provider did not report its base URL")

    def stop(self) -> str | None:
        failure: str | None = None
        if self._process is not None:
            if self._process.poll() is None:
                self._process.terminate()
                try:
                    self._process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    self._process.kill()
                    self._process.wait(timeout=10)
            if self._process.returncode not in (0, -15):
                failure = f"provider exited with {self._process.returncode}"
            if self._process.stdout is not None:
                output = self._process.stdout.read()
                if output:
                    print(output, end="")
            self._process = None
        if self._stderr is not None:
            self._stderr.close()
            self._stderr = None
        stderr = self._read_stderr()
        if stderr:
            print(stderr, end="", file=sys.stderr)
        return failure

    def _read_stderr(self) -> str:
        if self._stderr is not None:
            self._stderr.flush()
        if not self._stderr_path.is_file():
            return ""
        return self._stderr_path.read_text(encoding="utf-8")
