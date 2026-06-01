#!/usr/bin/env python3
"""Local HTTP/1.x socket workload for AcTrail payload capture."""

from __future__ import annotations

import os
import socket
import sys
import threading
import time


HTTP_BODY = b'{"source":"actrail-http","kind":"socket-plaintext"}'
HTTP_RESPONSE_BODY = b"actrail-http-ok"


def main() -> int:
    print(f"agent_pid={os.getpid()}", flush=True)
    print("waiting_for=go", flush=True)
    control = sys.stdin.readline()
    if control != "go\n":
        raise RuntimeError(f'expected control line "go", got {control!r}')

    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("127.0.0.1", 0))
    server.listen(2)
    host, port = server.getsockname()
    print(f"http_listen={host}:{port}", flush=True)

    thread = threading.Thread(target=serve_connections, args=(server,), daemon=False)
    thread.start()
    run_non_http_client(host, port)
    response = run_http_client(host, port)
    thread.join(timeout=5.0)
    if thread.is_alive():
        raise RuntimeError("server thread did not finish")
    server.close()

    print(f"http_response={response.decode('utf-8', errors='replace')}", flush=True)
    print("workload complete", flush=True)
    time.sleep(0.2)
    return 0


def serve_connections(server: socket.socket) -> None:
    with server.accept()[0] as connection:
        data = connection.recv(4096)
        if data != b"actrail-non-http\n":
            raise RuntimeError(f"unexpected non-http payload: {data!r}")
    print("non_http_socket_exchange=complete", flush=True)

    with server.accept()[0] as connection:
        request = connection.recv(4096)
        if b"POST /plain-http HTTP/1.1" not in request:
            raise RuntimeError(f"unexpected HTTP request: {request!r}")
        response = (
            b"HTTP/1.1 200 OK\r\n"
            b"Content-Type: text/plain\r\n"
            + f"Content-Length: {len(HTTP_RESPONSE_BODY)}\r\n".encode("ascii")
            + b"\r\n"
            + HTTP_RESPONSE_BODY
        )
        connection.sendall(response)


def run_non_http_client(host: str, port: int) -> None:
    with socket.create_connection((host, port), timeout=5.0) as client:
        client.sendall(b"actrail-non-http\n")


def run_http_client(host: str, port: int) -> bytes:
    request = (
        b"POST /plain-http HTTP/1.1\r\n"
        b"Host: local.actrail\r\n"
        b"Content-Type: application/json\r\n"
        b"Authorization: Bearer actrail-http-example\r\n"
        + f"Content-Length: {len(HTTP_BODY)}\r\n".encode("ascii")
        + b"\r\n"
        + HTTP_BODY
    )
    with socket.create_connection((host, port), timeout=5.0) as client:
        client.sendall(request)
        return client.recv(4096)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"http payload workload failed: {error}", file=sys.stderr)
        raise SystemExit(1)
