#!/usr/bin/env python3
"""Minimal allow-listed HTTP CONNECT proxy for the Kata OpenCode smoke."""

from __future__ import annotations

import argparse
import socket
import socketserver
import sys
import threading
from pathlib import Path


MAX_HEADER_BYTES = 16 * 1024


class ConnectProxy(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True

    def __init__(
        self,
        address: tuple[str, int],
        allowed_hosts: tuple[str, ...],
        allowed_port: int,
        log_path: Path | None,
    ) -> None:
        self.allowed_hosts = frozenset(host.lower() for host in allowed_hosts)
        self.allowed_port = allowed_port
        self.log_path = log_path
        super().__init__(address, ConnectHandler)

    def record(self, message: str) -> None:
        print(message, flush=True)
        if self.log_path is not None:
            with self.log_path.open("a", encoding="utf-8") as log:
                log.write(message + "\n")


class ConnectHandler(socketserver.BaseRequestHandler):
    server: ConnectProxy

    def handle(self) -> None:
        try:
            header = self._read_header()
            host, port = self._parse_connect(header)
            if (
                host.lower() not in self.server.allowed_hosts
                or port != self.server.allowed_port
            ):
                self.server.record(f"connect_proxy_denied={host}:{port}")
                self.request.sendall(b"HTTP/1.1 403 Forbidden\r\n\r\n")
                return
            upstream = socket.create_connection((host, port), timeout=15)
        except (OSError, ValueError):
            self._send_bad_gateway()
            return
        self.server.record(f"connect_proxy_tunnel={host}:{port}")
        with upstream:
            upstream.settimeout(None)
            self.request.sendall(
                b"HTTP/1.1 200 Connection Established\r\n"
                b"Proxy-Agent: actrail-v2-connect\r\n\r\n"
            )
            self._relay(upstream)

    def _read_header(self) -> bytes:
        content = bytearray()
        while b"\r\n\r\n" not in content:
            chunk = self.request.recv(4096)
            if not chunk:
                raise ValueError("client closed before CONNECT header")
            content.extend(chunk)
            if len(content) > MAX_HEADER_BYTES:
                raise ValueError("CONNECT header is too large")
        return bytes(content)

    @staticmethod
    def _parse_connect(header: bytes) -> tuple[str, int]:
        try:
            request_line = header.split(b"\r\n", 1)[0].decode("ascii")
            method, authority, protocol = request_line.split(" ")
            host, raw_port = authority.rsplit(":", 1)
            port = int(raw_port)
        except (UnicodeDecodeError, ValueError) as error:
            raise ValueError("invalid CONNECT request") from error
        if method != "CONNECT" or not protocol.startswith("HTTP/1.") or not host:
            raise ValueError("only HTTP CONNECT is supported")
        return host, port

    def _relay(self, upstream: socket.socket) -> None:
        def copy(source: socket.socket, target: socket.socket) -> None:
            while True:
                try:
                    content = source.recv(65536)
                    if not content:
                        break
                    target.sendall(content)
                except OSError:
                    break
            try:
                target.shutdown(socket.SHUT_WR)
            except OSError:
                pass

        request_to_upstream = threading.Thread(
            target=copy,
            args=(self.request, upstream),
            daemon=True,
        )
        request_to_upstream.start()
        copy(upstream, self.request)
        request_to_upstream.join(timeout=1)

    def _send_bad_gateway(self) -> None:
        try:
            self.request.sendall(b"HTTP/1.1 502 Bad Gateway\r\n\r\n")
        except OSError:
            pass


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser()
    result.add_argument("--bind-host", default="127.0.0.1")
    result.add_argument("--bind-port", type=int, required=True)
    result.add_argument(
        "--allow-host",
        action="append",
        dest="allowed_hosts",
        default=None,
        help="HTTPS host to allow; may be repeated",
    )
    result.add_argument("--allow-port", type=int, default=443)
    result.add_argument("--log-path", type=Path)
    return result


def main() -> int:
    arguments = parser().parse_args()
    if not 1 <= arguments.bind_port <= 65535:
        raise SystemExit("--bind-port must be between 1 and 65535")
    if not 1 <= arguments.allow_port <= 65535:
        raise SystemExit("--allow-port must be between 1 and 65535")
    allowed_hosts = tuple(
        arguments.allowed_hosts
        or ("opencode.ai", "models.opencode.ai")
    )
    with ConnectProxy(
        (arguments.bind_host, arguments.bind_port),
        allowed_hosts,
        arguments.allow_port,
        arguments.log_path,
    ) as proxy:
        proxy.record(
            f"connect_proxy_ready={arguments.bind_host}:{arguments.bind_port}"
        )
        try:
            proxy.serve_forever(poll_interval=0.1)
        except KeyboardInterrupt:
            return 0
    return 0


if __name__ == "__main__":
    sys.exit(main())
