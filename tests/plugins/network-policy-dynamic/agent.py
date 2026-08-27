#!/usr/bin/env python3
"""Long-lived connect workload for dynamic network-policy updates."""

from __future__ import annotations

import errno
import os
import socket
import sys


class ConnectAgent:
    def run(self) -> int:
        print(f"agent_pid={os.getpid()}", flush=True)
        for raw in sys.stdin:
            parts = raw.strip().split()
            if parts == ["quit"]:
                return 0
            if len(parts) == 3 and parts[0] == "connect":
                self._report(parts[2], self._connect(int(parts[1]), parts[2]))
                continue
            print(f"agent_error=invalid command {raw.strip()!r}", flush=True)
            return 2
        return 0

    @staticmethod
    def _connect(port: int, marker: str) -> str:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=5.0) as stream:
                stream.sendall(marker.encode("utf-8"))
                if stream.recv(64) != b"ok":
                    return "bad_response"
                return "ok"
        except PermissionError:
            return "permission_denied"
        except OSError as error:
            if error.errno == errno.EPERM:
                return "permission_denied"
            return f"os_error_{error.errno}"

    @staticmethod
    def _report(label: str, result: str) -> None:
        print(f"{label}={result}", flush=True)


if __name__ == "__main__":
    raise SystemExit(ConnectAgent().run())
