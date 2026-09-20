#!/usr/bin/env python3
"""Send a foreign-trace TLS-sync payload and verify its audited rejection."""

from __future__ import annotations

import os
import socket
import sqlite3
import struct
import sys
import time
from pathlib import Path


class ForeignTracePayload:
    def __init__(self, trace_id: int):
        self.trace_id = trace_id

    @staticmethod
    def _field(value: bytes) -> bytes:
        return struct.pack("<I", len(value)) + value

    def frame(self) -> bytes:
        # Match tls_payload_sync::protocol::{FrameHeader, FrameCodec}.
        stat = Path("/proc/self/stat").read_text(encoding="utf-8")
        start_ticks = int(stat.rsplit(")", 1)[1].split()[19])
        namespace = os.readlink("/proc/self/ns/pid").encode()
        body = b"".join(
            (
                struct.pack("<QIQ", self.trace_id, os.getpid(), start_ticks),
                self._field(namespace),
                b"\x00",  # Outbound.
                self._field(b"peer-e2e"),
                self._field(b"injection"),
                struct.pack("<QQ", 1, 1),
                self._field(b"hi"),
            )
        )
        return struct.pack("<2sBBI", b"AT", 1, 1, len(body)) + body

    def send(self, socket_path: str) -> None:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
            client.settimeout(2)
            client.connect(socket_path)
            client.sendall(self.frame())
            client.shutdown(socket.SHUT_WR)
            try:
                while client.recv(4096):
                    pass
            except socket.timeout:
                pass

    def verify_rejected(self, daemon_log: Path, database: Path, offset: int) -> str:
        rejection_reason = f"is not authorized for trace trace-{self.trace_id}"
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            with daemon_log.open("rb") as log:
                log.seek(offset)
                audit = log.read().decode("utf-8", errors="replace")
            if any(
                "closed rejected TLS-sync peer" in line and rejection_reason in line
                for line in audit.splitlines()
            ):
                break
            time.sleep(0.2)
        else:
            raise RuntimeError("foreign TLS payload injection lacked an audited rejection")
        with sqlite3.connect(database) as connection:
            forged = int(
                connection.execute(
                    "SELECT COUNT(*) FROM payload_segments "
                    "WHERE trace_id = ? AND library = 'peer-e2e' AND symbol = 'injection'",
                    (self.trace_id,),
                ).fetchone()[0]
            )
        if forged != 0:
            raise RuntimeError("foreign TLS payload reached another container trace")
        return rejection_reason


if __name__ == "__main__":
    ForeignTracePayload(int(sys.argv[1])).send(sys.argv[2])
