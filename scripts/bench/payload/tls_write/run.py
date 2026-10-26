"""Real OpenSSL write completion audit against an existing isolated daemon."""
from __future__ import annotations

import argparse
import json
import re
import socket
import sqlite3
import ssl
import struct
import subprocess
import threading
import time
import tomllib
from pathlib import Path


class Peer:
    def __init__(self, directory, name):
        self.failure = name == "failure"
        self.expected = 131072 if name == "limited" else 16384
        self.error = None
        self.received = 0
        self.listener = self.listen()
        self.control = self.listen()
        self.context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self.context.load_cert_chain(directory / "cert.pem", directory / "key.pem")
        self.thread = threading.Thread(target=self.serve, daemon=True)

    @staticmethod
    def listen():
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        listener.settimeout(30)
        return listener

    def serve(self):
        try:
            raw, _ = self.listener.accept()
            raw.settimeout(30)
            with self.context.wrap_socket(raw, server_side=True) as connection:
                if self.failure:
                    with self.control.accept()[0] as control:
                        connection.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
                        connection.close()
                        control.sendall(b"R")
                else:
                    while self.received < self.expected:
                        chunk = connection.recv(65536)
                        if not chunk:
                            raise RuntimeError("TLS peer closed before expected bytes")
                        self.received += len(chunk)
                    with self.control.accept()[0] as control:
                        control.sendall(b"A")
        except BaseException as error:
            self.error = error
        finally:
            self.listener.close()
            self.control.close()


class Audit:
    def __init__(self, args):
        self.args = args
        self.out = args.out.resolve()
        self.report = {"status": "running", "scope": "functional TLS write audit", "cases": []}

    def query(self, sql, parameters=()):
        with sqlite3.connect(f"file:{self.args.database.resolve()}?mode=ro", uri=True) as db:
            db.row_factory = sqlite3.Row
            return [dict(row) for row in db.execute(sql, parameters)]

    def run(self):
        self.out.mkdir(parents=True, exist_ok=False)
        try:
            config = tomllib.loads(self.args.config.read_text())
            if config["payload"]["tls"]["capture_backend"] != "bpf-copy":
                raise RuntimeError("requires bpf-copy configuration")
            if not config["semantic_retention"]["l4_payload"]["enabled"]:
                raise RuntimeError("requires L4 segment storage")
            if config["payload"]["tls"]["max_operation_bytes"] != 65535:
                raise RuntimeError("requires explicit 65535-byte TLS operation limit")
            self.report["config"] = str(self.args.config.resolve())
            (self.out / "resolved.toml").write_text(self.args.config.read_text())
            executable = self.out / "client"
            subprocess.run(["cc", "-O2", "-Wall", "-Wextra", str(Path(__file__).with_name("client.c")),
                            "-o", str(executable), "-lssl", "-lcrypto"], check=True)
            linkage = subprocess.run(["ldd", str(executable)], capture_output=True, text=True, check=True)
            (self.out / "ldd.txt").write_text(linkage.stdout)
            subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                            "-keyout", str(self.out / "key.pem"), "-out", str(self.out / "cert.pem"),
                            "-days", "1", "-subj", "/CN=localhost"], check=True,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30)
            for name in ("short", "limited", "failure"):
                self.case(executable, name)
            self.report["status"] = "passed"
        except BaseException as error:
            self.report.update(status="failed", error=f"{type(error).__name__}: {error}")
            raise
        finally:
            (self.out / "audit.json").write_text(json.dumps(self.report, indent=2) + "\n")

    def case(self, executable, name):
        peer = Peer(self.out, name)
        port, control = peer.listener.getsockname()[1], peer.control.getsockname()[1]
        before = self.query("SELECT COALESCE(MAX(trace_id),0) AS id FROM traces")[0]["id"]
        log_offset = self.args.daemon_log.stat().st_size
        command = [str(self.args.ctl.resolve()), "--config", str(self.args.config.resolve()), "launch", "--",
                   str(executable), str(port), str(control), name]
        peer.thread.start()
        try:
            result = subprocess.run(command, capture_output=True, text=True, timeout=40)
        finally:
            peer.thread.join(timeout=31)
        (self.out / f"{name}.stdout").write_text(result.stdout)
        (self.out / f"{name}.stderr").write_text(result.stderr)
        if result.returncode or peer.error or peer.thread.is_alive():
            raise RuntimeError(f"{name} client={result.returncode} peer={peer.error}")
        attempts = [json.loads(line) for line in result.stdout.splitlines() if line.startswith('{"pid":')]
        if not attempts:
            raise RuntimeError("no actual SSL_write observations")
        deadline = time.monotonic() + 30
        while True:
            traces = self.query("SELECT t.trace_id,p.process_id,p.host_pid,p.host_start_ticks FROM traces t "
                                "JOIN processes p ON p.process_id=t.root_process_id WHERE t.trace_id>?", (before,))
            with self.args.daemon_log.open() as source:
                source.seek(log_offset)
                log = source.read()
            if len(traces) == 1 and f"trace_finalization completed trace_id=trace-{traces[0]['trace_id']}" in log:
                break
            if time.monotonic() >= deadline:
                raise RuntimeError("trace finalization missing")
            time.sleep(0.05)
        (self.out / f"{name}.daemon.log").write_text(log)
        identity = traces[0]
        if any(attempt["pid"] != identity["host_pid"] for attempt in attempts):
            raise RuntimeError("client PID does not match recorded trace root")
        captures = re.findall(r"direct_capture operation_id=(\d+) pid=(\d+) generation=(\d+) offset=(\d+) bytes=(\d+)", log)
        operations = []
        generations = set()
        for operation, pid, generation, offset, size in captures:
            if int(pid) == identity["host_pid"] and operation not in operations:
                operations.append(operation)
                generations.add(int(generation))
        if len(operations) != len(attempts):
            raise RuntimeError(f"SSL_write/direct operation count mismatch: {len(attempts)}/{len(operations)}")
        if len(generations) != 1 or 0 in generations:
            raise RuntimeError("missing or inconsistent producer process generation")
        rows = self.query("SELECT * FROM payload_segments WHERE trace_id=? AND process_id=? "
                          "AND (segment_meta & 12)=0 AND (segment_meta & 1)=0 ORDER BY sequence",
                          (identity["trace_id"], identity["process_id"]))
        observations = []
        if any(row["symbol"] != "SSL_write" for row in rows):
            raise RuntimeError("unexpected TLS write route")
        for attempt, operation in zip(attempts, operations):
            segments = [row for row in rows if row["operation_id"] == int(operation)]
            if attempt["result"] > 0:
                captured = min(attempt["result"], 65535)
                truncation = 2 if attempt["result"] > captured else 0
                if not segments or sum(row["captured_size"] for row in segments) != captured:
                    raise RuntimeError("successful write captured byte count mismatch")
                if any(row["operation_original_size"] != attempt["result"] or
                       row["operation_captured_size"] != captured or
                       (row["segment_meta"] >> 4) & 3 != 1 or
                       (row["segment_meta"] >> 6) & 3 != truncation for row in segments):
                    raise RuntimeError("write completion/truncation metadata mismatch")
                if sum(row["original_size"] for row in segments) != attempt["result"]:
                    raise RuntimeError("segment missing byte count duplicated or lost")
            elif segments or f"drop_failed operation_id={operation}" not in log:
                raise RuntimeError("failed SSL_write lacks drop evidence or has stored payload")
            observations.append(dict(**attempt, operation_id=operation, segments=len(segments)))
        self.report["cases"].append(dict(name=name, command=command, identity=identity,
                                         producer_generation=next(iter(generations)),
                                         peer_received=peer.received, observations=observations))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for argument in ("out", "ctl", "config", "database", "daemon-log"):
        parser.add_argument(f"--{argument}", type=Path, required=True)
    Audit(parser.parse_args()).run()
