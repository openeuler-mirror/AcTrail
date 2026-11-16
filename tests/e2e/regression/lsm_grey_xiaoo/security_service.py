#!/usr/bin/env python3
"""Independent local authorization service with observable per-request decisions."""
from __future__ import annotations

import argparse
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


class SecurityService(ThreadingHTTPServer):
    def __init__(self, state: Path):
        super().__init__(("127.0.0.1", 0), RequestHandler)
        self.state = state
        self.requests: list[dict] = []
        self.delay = 0.0
        self.lock = threading.Lock()
        self.target_requests: list[str] = []
        self.reuse_executables = False
        self.collision_swapped = False
        self.management_requests: list[str] = []

    def analyze(self, request: dict, endpoint: str) -> dict:
        with self.lock:
            ordinal = len(self.requests) + 1
            verdict = "allow" if request["operation"] == "read" or ordinal % 2 else "deny"
            if request["operation"] == "exec":
                argv = request["argv"]
                pairs = list(zip(argv, argv[1:]))
                verdict = "allow" if ("-X", "GET") in pairs and ("-X", "POST") not in pairs else "deny"
                if len(argv) >= 4 and argv[1] == "--argv-probe":
                    verdict = "allow"
            reuse = self.reuse_executables and request["operation"] == "exec"
            if reuse:
                verdict = "allow" if Path(request["executable"]).name == "cat" else "deny"
            if "collision-" in request["target"]:
                verdict = "allow" if (endpoint == "/analyze") != self.collision_swapped else "deny"
            answer = {"verdict": verdict, "reason": f"security-service-request-{ordinal}"}
            if reuse:
                answer["reuse"] = "executable"
            self.requests.append({**request, "endpoint": endpoint, "ordinal": ordinal, **answer})
            self.state.write_text(json.dumps(self.requests, indent=2) + "\n")
            delay = self.delay
        time.sleep(delay)
        return answer


class RequestHandler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        self.record_target()

    def record_target(self) -> None:
        if self.path != "/target":
            self.send_error(404)
            return
        with self.server.lock:
            self.server.target_requests.append(self.command)
            self.server.state.with_name("target-requests.json").write_text(json.dumps(self.server.target_requests))
        body = b"CURL_TARGET_OK"
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_PUT(self) -> None:
        self.do_POST()

    def do_POST(self) -> None:
        if self.path == "/target":
            self.record_target()
            return
        document = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if self.path == "/management":
            self.management(document["mode"])
            return
        if self.path in ("/analyze", "/analyze/numeric"):
            if self.command != "PUT" or self.headers.get("X-Decision-Client") != "wasm-fixture":
                self.send_error(400)
                return
            result = self.server.analyze(document, self.path)
        elif self.path == "/collision":
            self.server.collision_swapped = bool(document["swapped"])
            result = {"ok": True}
        elif self.path == "/delay":
            self.server.delay = float(document["seconds"])
            result = {"ok": True}
        elif self.path == "/reuse":
            with self.server.lock:
                self.server.reuse_executables = bool(document["enabled"])
            result = {"ok": True}
        else:
            self.send_error(404)
            return
        body = json.dumps(result).encode()
        self.send_response(202 if self.path in ("/analyze", "/analyze/numeric") else 200)
        self.send_header("X-Decision-Source", "mock-service")
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except BrokenPipeError:
            pass  # A deadline may expire while this demonstration service computes.

    def management(self, mode: str) -> None:
        with self.server.lock:
            self.server.management_requests.append(mode)
            self.server.state.with_name("management-requests.json").write_text(json.dumps(self.server.management_requests))
        if mode == "slow-headers":
            time.sleep(2)
        body = b"MANAGEMENT_HTTP_OK"
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.flush()
        if mode == "slow-body":
            time.sleep(2)
        try:
            self.wfile.write(body)
        except BrokenPipeError:
            pass


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--state", required=True, type=Path)
    args = parser.parse_args()
    server = SecurityService(args.state)
    print(f"http://127.0.0.1:{server.server_port}", flush=True)
    server.serve_forever()
