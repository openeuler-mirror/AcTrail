#!/usr/bin/env python3
"""Local HTTPS/HTTP1 curl target for AcTrail payload capture E2E."""

from __future__ import annotations

import argparse
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


@dataclass(frozen=True)
class TargetConfig:
    bind_host: str
    listen_port: int
    listen_backlog: int
    socket_timeout_seconds: float
    child_timeout_seconds: float
    post_workload_hold_seconds: float
    cert_directory: Path
    cert_key_algorithm: str
    cert_subject: str
    cert_valid_days: int
    request_path: str
    request_body: str
    authorization_header: str
    response_body: str


def main() -> int:
    if len(sys.argv) > 1 and sys.argv[1] == SERVER_MODE:
        return server_main(sys.argv[2:])

    args = parse_args()
    target = load_target_config(args.target_config)
    certificate = ensure_certificate(target)
    print_attach_command(os.getpid(), args.config)
    input("press Enter after actrailctl reports the trace is Active...")
    run_workload(target, certificate)
    print("tls curl workload complete", flush=True)
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run a local HTTPS/HTTP1 curl workload after AcTrail attaches."
    )
    parser.add_argument(
        "--config",
        required=True,
        help="actraild operator config path printed in the attach command",
    )
    parser.add_argument(
        "--target-config",
        required=True,
        help="key-value workload config for the local HTTP/1.1 target",
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
    cert_path = target.cert_directory / "actrail-local.crt"
    key_path = target.cert_directory / "actrail-local.key"
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


def print_attach_command(pid: int, config_path: str) -> None:
    print("copy this command in another terminal:", flush=True)
    print(
        f"./target/release/actrailctl track-add --config {config_path} --pid {pid}",
        flush=True,
    )


def run_workload(target: TargetConfig, certificate: tuple[Path, Path]) -> None:
    server = start_server_child(target, certificate)
    try:
        server_port = read_server_port(server, target.child_timeout_seconds)
        print(f"tls_server_pid={server.pid} listen={target.bind_host}:{server_port}", flush=True)
        curl = start_curl_child(target, server_port)
        print(f"curl_pid={curl.pid}", flush=True)
        wait_child("curl", curl, target.child_timeout_seconds)
        wait_child("tls server", server, target.child_timeout_seconds)
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
        "--http1.1",
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
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(args.cert_path, args.key_path)

    listener = socket.socket()
    listener.settimeout(args.socket_timeout_seconds)
    listener.bind((args.bind_host, args.listen_port))
    listener.listen(args.listen_backlog)
    print(listener.getsockname()[1], flush=True)

    with listener:
        raw_conn, _ = listener.accept()
        with context.wrap_socket(raw_conn, server_side=True) as conn:
            conn.settimeout(args.socket_timeout_seconds)
            _ = read_http_request(conn)
            body = args.response_body.encode()
            response = (
                b"HTTP/1.1 200 OK\r\n"
                + b"Content-Type: application/json\r\n"
                + f"Content-Length: {len(body)}\r\n".encode()
                + b"Connection: close\r\n\r\n"
                + body
            )
            conn.sendall(response)
    return 0


def parse_server_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--bind-host", required=True)
    parser.add_argument("--listen-port", type=int, required=True)
    parser.add_argument("--listen-backlog", type=int, required=True)
    parser.add_argument("--socket-timeout-seconds", type=float, required=True)
    parser.add_argument("--response-body", required=True)
    parser.add_argument("--cert-path", required=True)
    parser.add_argument("--key-path", required=True)
    return parser.parse_args(argv)


def read_http_request(conn: ssl.SSLSocket) -> bytes:
    chunks: list[bytes] = []
    while True:
        chunk = conn.recv(4096)
        if not chunk:
            break
        chunks.append(chunk)
        joined = b"".join(chunks)
        header_end = joined.find(b"\r\n\r\n")
        if header_end < 0:
            continue
        content_length = parse_content_length(joined[:header_end])
        body_start = header_end + len(b"\r\n\r\n")
        if len(joined) - body_start >= content_length:
            return joined
    return b"".join(chunks)


def parse_content_length(headers: bytes) -> int:
    for line in headers.split(b"\r\n"):
        name, separator, value = line.partition(b":")
        if separator and name.lower() == b"content-length":
            return int(value.strip())
    return 0


if __name__ == "__main__":
    sys.exit(main())
