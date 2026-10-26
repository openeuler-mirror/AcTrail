"""Real TLS HTTP/2 fixture with explicitly coalesced DATA/RST writes."""

from __future__ import annotations

import json
import socket
import ssl
import subprocess
import threading
from pathlib import Path


class Http2WireServer:
    PREFACE = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"

    def __init__(self, out: Path, cases):
        self.out, self.cases = out, cases
        self.cert, self.key = out / "cert.pem", out / "key.pem"
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                        "-keyout", str(self.key), "-out", str(self.cert), "-days", "1",
                        "-subj", "/CN=localhost", "-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1"],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30)
        self.context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self.context.load_cert_chain(self.cert, self.key)
        self.context.set_alpn_protocols(["h2"])
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(1)
        self.listener.settimeout(1)
        self.port = self.listener.getsockname()[1]
        self.stopping = threading.Event()
        self.errors = []
        self.sessions = []
        self.thread = threading.Thread(target=self.serve, daemon=True)

    @staticmethod
    def frame(kind, flags, stream, payload=b""):
        return len(payload).to_bytes(3, "big") + bytes([kind, flags]) + stream.to_bytes(4, "big") + payload

    @staticmethod
    def read_exact(connection, length):
        data = bytearray()
        while len(data) < length:
            part = connection.recv(length - len(data))
            if not part:
                raise EOFError("HTTP/2 peer closed")
            data.extend(part)
        return bytes(data)

    def start(self):
        self.thread.start()

    def stop(self):
        self.stopping.set()
        self.thread.join(timeout=25)
        self.listener.close()
        if self.thread.is_alive():
            raise RuntimeError("HTTP/2 fixture did not stop")
        (self.out / "server.json").write_text(json.dumps({"sessions": self.sessions, "errors": self.errors}, indent=2))
        if self.errors:
            raise RuntimeError(f"HTTP/2 server failed: {self.errors}")

    def serve(self):
        while not self.stopping.is_set():
            try:
                peer, _ = self.listener.accept()
            except TimeoutError:
                continue
            try:
                with self.context.wrap_socket(peer, server_side=True) as connection:
                    connection.settimeout(20)
                    self.exchange(connection)
            except BaseException as error:
                self.errors.append(str(error))

    def exchange(self, connection):
        if connection.selected_alpn_protocol() != "h2" or self.read_exact(connection, 24) != self.PREFACE:
            raise RuntimeError("expected a real HTTP/2 connection preface")
        connection.sendall(self.frame(4, 0, 0))
        bodies, complete = {}, {}
        while len(complete) < len(self.cases):
            header = self.read_exact(connection, 9)
            size, kind, flags = int.from_bytes(header[:3], "big"), header[3], header[4]
            stream = int.from_bytes(header[5:], "big") & 0x7fffffff
            data = self.read_exact(connection, size)
            if kind == 4 and not flags & 1:
                connection.sendall(self.frame(4, 1, 0))
            elif kind == 0:
                if flags & 8:
                    raise RuntimeError("fixture expects unpadded client DATA")
                bodies.setdefault(stream, bytearray()).extend(data)
                if size:
                    update = size.to_bytes(4, "big")
                    connection.sendall(self.frame(8, 0, 0, update) + self.frame(8, 0, stream, update))
                if flags & 1:
                    request = json.loads(bodies[stream])
                    case = next(case for case in self.cases if case["request"] == request)
                    complete[stream] = case
        # All streams are active before any response. Interleave response fragments.
        batch = bytearray()
        delayed = []
        evidence = []
        for stream, case in complete.items():
            batch.extend(self.frame(1, 4, stream, b"\x88"))  # HPACK indexed :status 200.
            if case["name"] in ("reset_coalesced", "ordinary_reset"):
                batch.extend(self.frame(0, 0, stream, case["response"].encode()))
                batch.extend(self.frame(3, 0, stream, (8).to_bytes(4, "big")))
            elif case["name"] == "reset_delayed":
                batch.extend(self.frame(0, 0, stream, case["response"].encode()))
                delayed.append(self.frame(3, 0, stream, (8).to_bytes(4, "big")))
            else:
                response = case["response"].encode()
                split = len(response) // 2
                batch.extend(self.frame(0, 0, stream, response[:split]))
                delayed.append(self.frame(0, 1, stream, response[split:]))
            evidence.append({"name": case["name"], "stream_id": stream, "request_bytes": len(bodies[stream])})
        (self.out / f"session-{len(self.sessions)}-first.frames").write_bytes(batch)
        connection.sendall(batch)
        # A PING acknowledgement proves the client consumed the first response batch.
        connection.sendall(self.frame(6, 0, 0, b"progress"))
        while True:
            header = self.read_exact(connection, 9)
            payload = self.read_exact(connection, int.from_bytes(header[:3], "big"))
            if header[3] == 6 and header[4] & 1 and payload == b"progress":
                break
        final_batch = b"".join(delayed)
        (self.out / f"session-{len(self.sessions)}-final.frames").write_bytes(final_batch)
        connection.sendall(final_batch)
        self.sessions.append(evidence)
        # Wait for the real client's GOAWAY, keeping TLS close separate from stream termination.
        try:
            while True:
                header = self.read_exact(connection, 9)
                self.read_exact(connection, int.from_bytes(header[:3], "big"))
                if header[3] == 7:
                    break
        except EOFError:
            pass
