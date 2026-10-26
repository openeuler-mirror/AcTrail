from __future__ import annotations

import select
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import TextIO

from .constants import CASE_CONSTANTS


class WorkloadProvider:
    """Use the shared HTTP streaming provider to request one real Bash workload."""

    def __init__(self, repo: Path, work_dir: Path, command: str, timeout: float):
        self._proxy = repo / "tests/v2/common/llm-http-proxy/provider_proxy.py"
        self._stderr_path = work_dir / "provider.stderr.log"
        self._command = command
        self._timeout = timeout
        self._process: subprocess.Popen[str] | None = None
        self._stderr: TextIO | None = None

    def start(self) -> str:
        self._stderr = self._stderr_path.open("w", encoding="utf-8")
        self._process = subprocess.Popen(
            [sys.executable, str(self._proxy), "--mode", "local-stream",
             "--bind-host", CASE_CONSTANTS.loopback_host, "--bind-port", str(CASE_CONSTANTS.bind_port),
             "--local-stream-response-text", "ACTRAIL_LSM_WORKLOAD_DONE",
             "--local-stream-reasoning-tokens", str(CASE_CONSTANTS.local_stream_reasoning_tokens),
             "--local-tool-command", self._command],
            text=True, stdout=subprocess.PIPE, stderr=self._stderr,
        )
        assert self._process.stdout is not None
        deadline = time.monotonic() + self._timeout
        while time.monotonic() < deadline:
            readable, _, _ = select.select(
                [self._process.stdout], [], [], max(0, deadline - time.monotonic())
            )
            if readable:
                line = self._process.stdout.readline()
                print(line, end="")
                if line.startswith("proxy_base_url="):
                    return line.split("=", 1)[1].strip()
            if self._process.poll() is not None:
                raise RuntimeError("LSM workload provider exited before readiness")
        raise RuntimeError("LSM workload provider readiness timed out")

    def stop(self) -> None:
        try:
            if self._process is not None:
                if self._process.poll() is None:
                    self._process.terminate()
                    try:
                        self._process.wait(timeout=self._timeout)
                    except subprocess.TimeoutExpired:
                        self._process.kill()
                        self._process.wait(timeout=self._timeout)
                if self._process.stdout is not None:
                    self._process.stdout.close()
                code = self._process.returncode
                self._process = None
                if code not in (0, -15):
                    raise RuntimeError(f"LSM workload provider exited with {code}")
        finally:
            if self._stderr is not None:
                self._stderr.close()
                self._stderr = None
            if self._stderr_path.is_file():
                print(self._stderr_path.read_text(encoding="utf-8"), file=sys.stderr, end="")


class TcpWitness:
    """An actual local peer records connections reaching userspace."""

    def __init__(self, timeout: float, poll_interval: float, *, ipv6: bool = CASE_CONSTANTS.init_ipv6):
        self._timeout = timeout
        self._poll_interval = poll_interval
        self._ipv6 = ipv6
        self._socket: socket.socket | None = None
        self._thread: threading.Thread | None = None
        self._stopped = threading.Event()
        self.requests: list[bytes] = []
        self.failure: str | None = None

    def start(self) -> int:
        self._socket = socket.socket(socket.AF_INET6 if self._ipv6 else socket.AF_INET, socket.SOCK_STREAM)
        self._socket.bind((CASE_CONSTANTS.ipv6_loopback_host if self._ipv6 else CASE_CONSTANTS.witness_host, 0))
        self._socket.listen()
        self._socket.settimeout(self._poll_interval)
        port = self._socket.getsockname()[1]
        self._thread = threading.Thread(target=self._serve, daemon=True)
        self._thread.start()
        return port

    def _serve(self) -> None:
        assert self._socket is not None
        try:
            while not self._stopped.is_set():
                try:
                    connection, _ = self._socket.accept()
                except socket.timeout:
                    continue
                with connection:
                    connection.settimeout(self._timeout)
                    payload = b""
                    expected_size = len(b"ACTRAIL_LSM_NETWORK")
                    while len(payload) < expected_size:
                        part = connection.recv(expected_size - len(payload))
                        if not part:
                            break
                        payload += part
                    self.requests.append(payload)
                    connection.sendall(payload)
        except OSError as error:
            if not self._stopped.is_set():
                self.failure = str(error)

    def stop(self) -> None:
        self._stopped.set()
        if self._socket is not None:
            self._socket.close()
        if self._thread is not None:
            self._thread.join(timeout=self._timeout)
            if self._thread.is_alive():
                raise RuntimeError("TCP witness did not stop")
        if self.failure:
            raise RuntimeError(f"TCP witness failed: {self.failure}")
