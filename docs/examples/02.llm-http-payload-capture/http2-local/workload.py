#!/usr/bin/env python3
"""Local HTTPS/HTTP2 curl target for AcTrail payload capture E2E."""

from __future__ import annotations

import argparse
import errno
import os
import select
import socket
import ssl
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

SERVER_MODE = "__server"

CONNECTION_PREFACE = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"
FRAME_HEADER_BYTES = 9
FRAME_TYPE_DATA = 0x0
FRAME_TYPE_HEADERS = 0x1
FRAME_TYPE_SETTINGS = 0x4
FRAME_TYPE_GOAWAY = 0x7
FLAG_END_STREAM = 0x1
FLAG_END_HEADERS = 0x4
FLAG_ACK = 0x1
HTTP2_NO_ERROR = 0
HTTP2_STREAM_ID_MASK = 0x7FFF_FFFF


@dataclass(frozen=True)
class TargetConfig:
    bind_host: str
    listen_port: int
    listen_backlog: int
    socket_timeout_seconds: float
    child_timeout_seconds: float
    post_workload_hold_seconds: float
    response_delay_seconds: float
    post_response_hold_seconds: float
    cert_directory: Path
    cert_key_algorithm: str
    cert_subject: str
    cert_valid_days: int
    request_path: str
    request_body: str
    authorization_header: str
    response_body: str


@dataclass(frozen=True)
class Frame:
    frame_type: int
    flags: int
    stream_id: int
    payload: bytes


def main() -> int:
    if len(sys.argv) > 1 and sys.argv[1] == SERVER_MODE:
        return server_main(sys.argv[2:])

    args = parse_args()
    target = load_target_config(args.target_config)
    certificate = ensure_certificate(target)
    if args.serve_only:
        return serve_foreground(target, certificate)
    run_workload(target, certificate)
    print("http2 payload workload complete", flush=True)
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run a local HTTPS/HTTP2 curl workload.")
    parser.add_argument(
        "--target-config",
        required=True,
        help="key-value workload config for the local HTTP/2 target",
    )
    parser.add_argument(
        "--serve-only",
        action="store_true",
        help="start only the local TLS+h2 server and print its port",
    )
    return parser.parse_args()


def load_target_config(path: str) -> TargetConfig:
    values = parse_key_values(Path(path))
    required = [
        "bind_host",
        "listen_port",
        "listen_backlog",
        "socket_timeout_seconds",
        "child_timeout_seconds",
        "post_workload_hold_seconds",
        "response_delay_seconds",
        "post_response_hold_seconds",
        "cert_directory",
        "cert_key_algorithm",
        "cert_subject",
        "cert_valid_days",
        "request_path",
        "request_body",
        "authorization_header",
        "response_body",
    ]
    missing = [key for key in required if key not in values]
    if missing:
        raise RuntimeError(f"missing target config keys: {', '.join(missing)}")
    unknown = sorted(key for key in values if key not in required)
    if unknown:
        raise RuntimeError(f"unknown target config keys: {', '.join(unknown)}")
    return TargetConfig(
        bind_host=values["bind_host"],
        listen_port=parse_int(values, "listen_port"),
        listen_backlog=parse_positive_int(values, "listen_backlog"),
        socket_timeout_seconds=parse_float(values, "socket_timeout_seconds"),
        child_timeout_seconds=parse_float(values, "child_timeout_seconds"),
        post_workload_hold_seconds=parse_float(values, "post_workload_hold_seconds"),
        response_delay_seconds=parse_float(values, "response_delay_seconds"),
        post_response_hold_seconds=parse_float(values, "post_response_hold_seconds"),
        cert_directory=Path(values["cert_directory"]),
        cert_key_algorithm=values["cert_key_algorithm"],
        cert_subject=values["cert_subject"],
        cert_valid_days=parse_positive_int(values, "cert_valid_days"),
        request_path=values["request_path"],
        request_body=values["request_body"],
        authorization_header=values["authorization_header"],
        response_body=values["response_body"],
    )


def parse_key_values(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for line_number, raw_line in enumerate(path.read_text().splitlines(), start=1):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if not separator:
            raise RuntimeError(f"{path}:{line_number}: expected key = value")
        key = key.strip()
        if key in values:
            raise RuntimeError(f"{path}:{line_number}: duplicate key {key}")
        values[key] = value.strip()
    return values


def parse_int(values: dict[str, str], key: str) -> int:
    value = int(values[key])
    if value < 0:
        raise RuntimeError(f"{key} must be non-negative")
    return value


def parse_positive_int(values: dict[str, str], key: str) -> int:
    value = int(values[key])
    if value <= 0:
        raise RuntimeError(f"{key} must be positive")
    return value


def parse_float(values: dict[str, str], key: str) -> float:
    value = float(values[key])
    if value <= 0:
        raise RuntimeError(f"{key} must be positive")
    return value


def ensure_certificate(target: TargetConfig) -> tuple[Path, Path]:
    target.cert_directory.mkdir(parents=True, exist_ok=True)
    cert_path = target.cert_directory / "actrail-http2-payload.crt"
    key_path = target.cert_directory / "actrail-http2-payload.key"
    if cert_path.exists() and key_path.exists():
        return cert_path, key_path

    command = [
        "openssl",
        "req",
        "-x509",
        "-newkey",
        target.cert_key_algorithm,
        "-nodes",
        "-days",
        str(target.cert_valid_days),
        "-subj",
        target.cert_subject,
        "-keyout",
        str(key_path),
        "-out",
        str(cert_path),
    ]
    subprocess.run(command, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return cert_path, key_path


def run_workload(target: TargetConfig, certificate: tuple[Path, Path]) -> None:
    server = start_server_child(target, certificate)
    try:
        server_port = read_server_port(server, target.child_timeout_seconds)
        print(f"http2_server_pid={server.pid} listen={target.bind_host}:{server_port}", flush=True)
        curl = start_curl_child(target, server_port)
        print(f"curl_pid={curl.pid}", flush=True)
        wait_child("curl", curl, target.child_timeout_seconds)
        wait_child("http2 server", server, target.child_timeout_seconds)
    finally:
        terminate_if_running(server, target.child_timeout_seconds)
    time.sleep(target.post_workload_hold_seconds)


def start_server_child(
    target: TargetConfig,
    certificate: tuple[Path, Path],
) -> subprocess.Popen[str]:
    cert_path, key_path = certificate
    return subprocess.Popen(
        [
            sys.executable,
            os.path.abspath(__file__),
            SERVER_MODE,
            "--bind-host",
            target.bind_host,
            "--listen-port",
            str(target.listen_port),
            "--listen-backlog",
            str(target.listen_backlog),
            "--socket-timeout-seconds",
            str(target.socket_timeout_seconds),
            "--response-body",
            target.response_body,
            "--response-delay-seconds",
            str(target.response_delay_seconds),
            "--post-response-hold-seconds",
            str(target.post_response_hold_seconds),
            "--cert-path",
            str(cert_path),
            "--key-path",
            str(key_path),
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


def start_curl_child(target: TargetConfig, server_port: int) -> subprocess.Popen[str]:
    url = f"https://{target.bind_host}:{server_port}{target.request_path}"
    command = [
        "curl",
        "--http2",
        "--silent",
        "--show-error",
        "--insecure",
        "--request",
        "POST",
        "--header",
        "Content-Type: application/json",
    ]
    if target.authorization_header:
        command.extend(["--header", f"Authorization: {target.authorization_header}"])
    command.extend(["--data", target.request_body, url])
    return subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)


def read_server_port(server: subprocess.Popen[str], timeout_seconds: float) -> int:
    if server.stdout is None:
        raise RuntimeError("server stdout is not available")
    ready, _, _ = select.select([server.stdout], [], [], timeout_seconds)
    if not ready:
        terminate_if_running(server, timeout_seconds)
        detail = child_output(server)
        raise RuntimeError(f"server did not publish its listen port before timeout: {detail}")
    raw = server.stdout.readline().strip()
    try:
        return int(raw)
    except ValueError as error:
        terminate_if_running(server, timeout_seconds)
        detail = child_output(server)
        raise RuntimeError(f"server published invalid listen port: {raw!r}: {detail}") from error


def wait_child(label: str, child: subprocess.Popen[str], timeout_seconds: float) -> None:
    try:
        stdout, stderr = child.communicate(timeout=timeout_seconds)
    except subprocess.TimeoutExpired as error:
        terminate_if_running(child, timeout_seconds)
        raise RuntimeError(f"{label} child did not exit before timeout") from error
    if child.returncode != 0:
        detail = format_child_output(stdout, stderr)
        raise RuntimeError(f"{label} child failed with exit code {child.returncode}: {detail}")
    if stdout.strip():
        print(f"{label}_stdout={stdout.strip()}", flush=True)


def terminate_if_running(child: subprocess.Popen[str], timeout_seconds: float) -> None:
    if child.poll() is not None:
        return
    child.terminate()
    try:
        child.wait(timeout=timeout_seconds)
    except subprocess.TimeoutExpired:
        child.kill()
        child.wait()


def child_output(child: subprocess.Popen[str]) -> str:
    stdout, stderr = child.communicate()
    return format_child_output(stdout, stderr)


def format_child_output(stdout: str, stderr: str) -> str:
    return "\n".join(part for part in [stdout.strip(), stderr.strip()] if part)


def server_main(argv: list[str]) -> int:
    args = parse_server_args(argv)
    return run_server(args)


def serve_foreground(target: TargetConfig, certificate: tuple[Path, Path]) -> int:
    cert_path, key_path = certificate
    return run_server(
        argparse.Namespace(
            bind_host=target.bind_host,
            listen_port=target.listen_port,
            listen_backlog=target.listen_backlog,
            socket_timeout_seconds=target.socket_timeout_seconds,
            response_body=target.response_body,
            response_delay_seconds=target.response_delay_seconds,
            post_response_hold_seconds=target.post_response_hold_seconds,
            cert_path=str(cert_path),
            key_path=str(key_path),
        )
    )


def run_server(args: argparse.Namespace) -> int:
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.set_alpn_protocols(["h2"])
    context.load_cert_chain(args.cert_path, args.key_path)

    listener = socket.socket()
    listener.settimeout(args.socket_timeout_seconds)
    listener.bind((args.bind_host, args.listen_port))
    listener.listen(args.listen_backlog)
    print(listener.getsockname()[1], flush=True)

    try:
        raw_conn, _ = listener.accept()
        with context.wrap_socket(raw_conn, server_side=True) as conn:
            conn.settimeout(args.socket_timeout_seconds)
            if conn.selected_alpn_protocol() != "h2":
                raise RuntimeError("client did not negotiate h2 over TLS ALPN")
            run_h2_exchange(conn, args.response_body.encode(), args.response_delay_seconds)
            time.sleep(args.post_response_hold_seconds)
    finally:
        close_listener(listener)
    return 0


def close_listener(listener: socket.socket) -> None:
    try:
        listener.close()
    except OSError as error:
        if error.errno != errno.EBADF:
            raise


def parse_server_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--bind-host", required=True)
    parser.add_argument("--listen-port", type=int, required=True)
    parser.add_argument("--listen-backlog", type=int, required=True)
    parser.add_argument("--socket-timeout-seconds", type=float, required=True)
    parser.add_argument("--response-body", required=True)
    parser.add_argument("--response-delay-seconds", type=float, required=True)
    parser.add_argument("--post-response-hold-seconds", type=float, required=True)
    parser.add_argument("--cert-path", required=True)
    parser.add_argument("--key-path", required=True)
    return parser.parse_args(argv)


def run_h2_exchange(
    conn: ssl.SSLSocket,
    response_body: bytes,
    response_delay_seconds: float,
) -> None:
    preface = read_exact(conn, len(CONNECTION_PREFACE))
    if preface != CONNECTION_PREFACE:
        raise RuntimeError("client did not send the HTTP/2 connection preface")
    send_frame(conn, FRAME_TYPE_SETTINGS, 0, 0, b"")

    response_stream_id: int | None = None
    while True:
        frame = read_frame(conn)
        if frame.frame_type == FRAME_TYPE_SETTINGS and frame.flags == 0:
            send_frame(conn, FRAME_TYPE_SETTINGS, FLAG_ACK, 0, b"")
        if frame.frame_type in (FRAME_TYPE_HEADERS, FRAME_TYPE_DATA):
            response_stream_id = frame.stream_id
        if frame.flags & FLAG_END_STREAM:
            break
    if response_stream_id is None:
        raise RuntimeError("client did not open a request stream")

    time.sleep(response_delay_seconds)
    send_frame(
        conn,
        FRAME_TYPE_HEADERS,
        FLAG_END_HEADERS,
        response_stream_id,
        hpack_status_200(),
    )
    send_frame(
        conn,
        FRAME_TYPE_DATA,
        FLAG_END_STREAM,
        response_stream_id,
        response_body,
    )
    send_goaway(conn, response_stream_id)


def send_goaway(conn: ssl.SSLSocket, last_stream_id: int) -> None:
    payload = (
        (last_stream_id & HTTP2_STREAM_ID_MASK).to_bytes(4, "big")
        + HTTP2_NO_ERROR.to_bytes(4, "big")
    )
    send_frame(conn, FRAME_TYPE_GOAWAY, 0, 0, payload)


def read_frame(conn: ssl.SSLSocket) -> Frame:
    header = read_exact(conn, FRAME_HEADER_BYTES)
    length = int.from_bytes(header[0:3], "big")
    frame_type = header[3]
    flags = header[4]
    stream_id = int.from_bytes(header[5:9], "big") & HTTP2_STREAM_ID_MASK
    return Frame(
        frame_type=frame_type,
        flags=flags,
        stream_id=stream_id,
        payload=read_exact(conn, length),
    )


def read_exact(conn: ssl.SSLSocket, size: int) -> bytes:
    chunks: list[bytes] = []
    remaining = size
    while remaining > 0:
        chunk = conn.recv(remaining)
        if not chunk:
            raise RuntimeError("connection closed before expected bytes arrived")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def send_frame(
    conn: ssl.SSLSocket,
    frame_type: int,
    flags: int,
    stream_id: int,
    payload: bytes,
) -> None:
    header = (
        len(payload).to_bytes(3, "big")
        + bytes([frame_type, flags])
        + (stream_id & HTTP2_STREAM_ID_MASK).to_bytes(4, "big")
    )
    conn.sendall(header + payload)


def hpack_status_200() -> bytes:
    return b"\x88"


if __name__ == "__main__":
    sys.exit(main())
