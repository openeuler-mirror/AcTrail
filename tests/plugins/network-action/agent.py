#!/usr/bin/env python3
"""Network-action ControlDecider E2E workload."""

from __future__ import annotations

import argparse
import errno
import os
import socket
import sys


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--allowed-host", default="127.0.0.1")
    parser.add_argument("--allowed-port", type=int, required=True)
    parser.add_argument("--denied-host", default="127.0.0.1")
    parser.add_argument("--denied-port", type=int, required=True)
    return parser.parse_args()


def connect_and_exchange(host: str, port: int, marker: bytes) -> bytes:
    with socket.create_connection((host, port), timeout=5.0) as stream:
        stream.sendall(marker)
        return stream.recv(64)


def run_allowed(host: str, port: int) -> None:
    response = connect_and_exchange(host, port, b"allowed")
    if response != b"ok":
        raise RuntimeError(f"unexpected allowed server response: {response!r}")
    print("allowed_connect=ok", flush=True)


def run_denied(host: str, port: int) -> None:
    try:
        connect_and_exchange(host, port, b"denied")
    except PermissionError:
        print("denied_connect=permission_denied", flush=True)
        return
    except OSError as error:
        if error.errno == errno.EPERM:
            print("denied_connect=permission_denied", flush=True)
            return
        raise
    raise RuntimeError("denied connect unexpectedly succeeded")


def main() -> int:
    args = parse_args()
    print(f"agent_pid={os.getpid()}", flush=True)
    control = sys.stdin.readline()
    if control.strip() != "go":
        print(f"unexpected_control={control!r}", flush=True)
        return 2
    run_allowed(args.allowed_host, args.allowed_port)
    run_denied(args.denied_host, args.denied_port)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
