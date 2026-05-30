#!/usr/bin/env python3
"""Manual attach target for AcTrail lifecycle and loopback network capture."""

from __future__ import annotations

import argparse
import os
import select
import socket
import subprocess
import sys
import time

SERVER_MODE = "__server"
CLIENT_MODE = "__client"


def main() -> int:
    if len(sys.argv) > 1 and sys.argv[1] == SERVER_MODE:
        return server_main(sys.argv[2:])
    if len(sys.argv) > 1 and sys.argv[1] == CLIENT_MODE:
        return client_main(sys.argv[2:])

    args = parse_args()
    print_attach_command(os.getpid(), args.config)
    input("press Enter after actrailctl reports the trace is Active...")
    run_workload(args)
    print("workload complete", flush=True)
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run child-process lifecycle plus loopback network workload for manual AcTrail verification."
    )
    parser.add_argument(
        "--config",
        default=os.environ.get(
            "ACTRAIL_TARGET_CONFIG",
            "docs/examples/01.quick-start/operator.conf",
        ),
        help="actraild operator config path printed in the attach command",
    )
    parser.add_argument(
        "--bind-host",
        default=os.environ.get("ACTRAIL_TARGET_BIND_HOST", "127.0.0.1"),
        help="loopback address used by the child server",
    )
    parser.add_argument(
        "--listen-port",
        type=int,
        default=int(os.environ.get("ACTRAIL_TARGET_LISTEN_PORT", "0")),
        help="server listen port; 0 asks the kernel to allocate one",
    )
    parser.add_argument(
        "--client-payload",
        default=os.environ.get("ACTRAIL_TARGET_CLIENT_PAYLOAD", "actrail-client-payload"),
        help="payload sent from the client child process",
    )
    parser.add_argument(
        "--server-payload",
        default=os.environ.get("ACTRAIL_TARGET_SERVER_PAYLOAD", "actrail-server-payload"),
        help="payload sent from the server child process",
    )
    parser.add_argument(
        "--socket-timeout-seconds",
        type=float,
        default=float(os.environ.get("ACTRAIL_TARGET_SOCKET_TIMEOUT_SECONDS", "5")),
        help="timeout for loopback socket operations",
    )
    parser.add_argument(
        "--child-hold-seconds",
        type=float,
        default=float(os.environ.get("ACTRAIL_TARGET_CHILD_HOLD_SECONDS", "1")),
        help="time child processes stay alive after their socket work completes",
    )
    parser.add_argument(
        "--post-workload-hold-seconds",
        type=float,
        default=float(os.environ.get("ACTRAIL_TARGET_POST_WORKLOAD_HOLD_SECONDS", "1")),
        help="time to keep the target process alive after workload completion",
    )
    return parser.parse_args()


def print_attach_command(pid: int, config_path: str) -> None:
    print("copy this command in another terminal:", flush=True)
    print(
        f"./target/release/actrailctl track-add --config {config_path} --pid {pid}",
        flush=True,
    )


def run_workload(args: argparse.Namespace) -> None:
    server = start_server_child(args)
    try:
        server_port = read_server_port(server, args.socket_timeout_seconds)
        print(f"server_pid={server.pid} listen={args.bind_host}:{server_port}", flush=True)
        client = start_client_child(args, server_port)
        print(f"client_pid={client.pid}", flush=True)
        wait_child("client", client, args.socket_timeout_seconds)
        wait_child("server", server, args.socket_timeout_seconds)
    finally:
        terminate_if_running(server, args.socket_timeout_seconds)
    time.sleep(args.post_workload_hold_seconds)


def start_server_child(args: argparse.Namespace) -> subprocess.Popen[str]:
    return subprocess.Popen(
        [
            sys.executable,
            os.path.abspath(__file__),
            SERVER_MODE,
            "--bind-host",
            args.bind_host,
            "--listen-port",
            str(args.listen_port),
            "--socket-timeout-seconds",
            str(args.socket_timeout_seconds),
            "--client-payload",
            args.client_payload,
            "--server-payload",
            args.server_payload,
            "--child-hold-seconds",
            str(args.child_hold_seconds),
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


def start_client_child(args: argparse.Namespace, server_port: int) -> subprocess.Popen[str]:
    return subprocess.Popen(
        [
            sys.executable,
            os.path.abspath(__file__),
            CLIENT_MODE,
            "--target-host",
            args.bind_host,
            "--target-port",
            str(server_port),
            "--socket-timeout-seconds",
            str(args.socket_timeout_seconds),
            "--client-payload",
            args.client_payload,
            "--server-payload",
            args.server_payload,
            "--child-hold-seconds",
            str(args.child_hold_seconds),
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


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
    listener = socket.socket()
    listener.settimeout(args.socket_timeout_seconds)
    listener.bind((args.bind_host, args.listen_port))
    listener.listen(1)
    print(listener.getsockname()[1], flush=True)

    expected_payload = args.client_payload.encode()
    response_payload = args.server_payload.encode()
    with listener:
        conn, _ = listener.accept()
        with conn:
            conn.settimeout(args.socket_timeout_seconds)
            received = conn.recv(len(expected_payload))
            if received != expected_payload:
                raise RuntimeError(f"unexpected client payload: {received!r}")
            conn.sendall(response_payload)
    time.sleep(args.child_hold_seconds)
    return 0


def client_main(argv: list[str]) -> int:
    args = parse_client_args(argv)
    client_payload = args.client_payload.encode()
    expected_payload = args.server_payload.encode()
    with socket.create_connection(
        (args.target_host, args.target_port),
        timeout=args.socket_timeout_seconds,
    ) as client:
        client.settimeout(args.socket_timeout_seconds)
        client.sendall(client_payload)
        received = client.recv(len(expected_payload))
        if received != expected_payload:
            raise RuntimeError(f"unexpected server payload: {received!r}")
    time.sleep(args.child_hold_seconds)
    return 0


def parse_server_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--bind-host", required=True)
    parser.add_argument("--listen-port", type=int, required=True)
    parser.add_argument("--socket-timeout-seconds", type=float, required=True)
    parser.add_argument("--client-payload", required=True)
    parser.add_argument("--server-payload", required=True)
    parser.add_argument("--child-hold-seconds", type=float, required=True)
    return parser.parse_args(argv)


def parse_client_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--target-host", required=True)
    parser.add_argument("--target-port", type=int, required=True)
    parser.add_argument("--socket-timeout-seconds", type=float, required=True)
    parser.add_argument("--client-payload", required=True)
    parser.add_argument("--server-payload", required=True)
    parser.add_argument("--child-hold-seconds", type=float, required=True)
    return parser.parse_args(argv)


if __name__ == "__main__":
    sys.exit(main())
