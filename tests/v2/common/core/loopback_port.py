from __future__ import annotations

import os
import socket
import struct


class _AutoPort:
    __slots__ = ()


AUTO = _AutoPort()


class LoopbackPortAllocator:
    """Choose an IPv4 loopback port reachable through the host TCP path."""

    def __init__(
        self,
        *,
        attempts: int,
        connect_timeout_seconds: float,
    ) -> None:
        if attempts <= 0:
            raise ValueError("loopback port attempts must be positive")
        if connect_timeout_seconds <= 0:
            raise ValueError("loopback port connect timeout must be positive")
        self._attempts = attempts
        self._connect_timeout_seconds = connect_timeout_seconds

    def allocate(self) -> int:
        attempted_ports: list[int] = []
        last_error: OSError | None = None
        for _ in range(self._attempts):
            with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
                listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                listener.settimeout(self._connect_timeout_seconds)
                listener.bind(("127.0.0.1", 0))
                listener.listen(1)
                port = int(listener.getsockname()[1])
                attempted_ports.append(port)
                try:
                    self._require_reachable(listener, port)
                except OSError as error:
                    last_error = error
                    continue
                return port
        raise RuntimeError(
            f"no reachable 127.0.0.1 TCP port after {self._attempts} "
            f"attempts ports={attempted_ports}: {last_error}"
        )

    def _require_reachable(self, listener: socket.socket, port: int) -> None:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as client:
            client.settimeout(self._connect_timeout_seconds)
            client.setsockopt(
                socket.SOL_SOCKET,
                socket.SO_LINGER,
                struct.pack("ii", 1, 0),
            )
            client.connect(("127.0.0.1", port))
            accepted, _ = listener.accept()
            accepted.close()


def resolve_test_port(
    environment_variable: str,
    *,
    fallback: int | _AutoPort = AUTO,
    attempts: int = 30,
    connect_timeout_seconds: float = 1.0,
) -> int:
    """Resolve an explicit test port or allocate a reachable loopback port."""

    if not environment_variable:
        raise ValueError("test port environment variable must be nonempty")
    configured = os.environ.get(environment_variable)
    if configured is not None:
        return _validate_port(configured, environment_variable)
    if fallback is not AUTO:
        return _validate_port(fallback, "fallback")
    return LoopbackPortAllocator(
        attempts=attempts,
        connect_timeout_seconds=connect_timeout_seconds,
    ).allocate()


def _validate_port(value: int | str, source: str) -> int:
    try:
        port = int(value)
    except (TypeError, ValueError) as error:
        raise ValueError(f"{source} must be an integer TCP port") from error
    if port < 1 or port > 65535:
        raise ValueError(f"{source} must be between 1 and 65535")
    return port
