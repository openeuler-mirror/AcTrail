#!/usr/bin/env python3
"""Capture a real agent's ordinary HTTP tool request and verify its default Web view."""
import argparse
from contextlib import closing
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import re
import shlex
import sqlite3
import subprocess
import sys
from threading import Thread

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from run import Acceptance, REPO


class OrdinaryHttpServer:
    path = "/ordinary-network-verification.txt"
    marker = "ACTRAIL_ORDINARY_HTTP_OK"

    def __init__(self):
        self.requests = []
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                owner.requests.append(self.path)
                body = (owner.marker + "\n").encode()
                self.send_response(200 if self.path == owner.path else 404)
                self.send_header("Content-Type", "text/plain; charset=utf-8")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *_args):
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self):
        return f"http://127.0.0.1:{self.server.server_port}{self.path}"

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


class NetworkAcceptance(Acceptance):
    def run(self):
        report = {"status": "failed"}
        server = OrdinaryHttpServer()
        try:
            self.runtime.prepare()
            task = self.work / "task"
            task.mkdir()
            output = task / "network-result.txt"
            command = shlex.join(["curl", "--fail", "--silent", "--show-error", "--noproxy", "*",
                                  "--output", str(output), server.url])
            prompt = (f"Run this exact command using your shell tool: {command}\n"
                      "Then read its output file with a tool and reply exactly NETWORK_E2E_OK. "
                      "Do not create or edit the output file yourself.")
            result = self.runtime.run(self.runtime.control_command(
                "launch", "--", self.args.agent.resolve(), "--cli", "run", "--max-turns", "6",
                "--prompt", prompt), timeout_seconds=self.args.agent_timeout, cwd=task)
            (self.work / "agent.log").write_text(result.output)
            if result.returncode or "NETWORK_E2E_OK" not in result.output:
                raise RuntimeError("real network agent task failed; inspect agent.log")
            traces = re.findall(r"trace trace-(\d+) entered Active", result.output)
            if len(traces) != 1:
                raise RuntimeError(f"expected one real trace, got {traces}")
            trace_id = int(traces[0])
            if server.path not in server.requests or output.read_text().strip() != server.marker:
                raise RuntimeError("real curl request or persisted response marker missing")
            stopped = self.runtime.stop()
            if stopped is None or stopped.returncode:
                raise RuntimeError("daemon failed to drain")
            with closing(sqlite3.connect(f"file:{self.work / 'data/actrail.sqlite'}?mode=ro", uri=True)) as database:
                rows = database.execute(
                    "SELECT ids.action_id,a.title,a.start_time FROM semantic_actions a "
                    "JOIN semantic_action_ids ids USING(action_key) "
                    "WHERE a.trace_id=? AND a.kind_code=108 AND a.action_valid_code=1 AND a.title=? "
                    "AND NOT EXISTS (SELECT 1 FROM semantic_action_links l "
                    "WHERE l.trace_id=a.trace_id AND l.child_action_key=a.action_key "
                    "AND l.valid=1 AND l.role_code IN (512,514)) ORDER BY a.start_time",
                    (trace_id, 'GET ' + server.path)).fetchall()
            if not rows:
                raise RuntimeError("real ordinary GET has no valid independent HTTP action")
            action_id, title, start = rows[0]
            expected = {"trace_id": trace_id, "action_id": action_id, "title": title,
                        "path": server.path, "start_time_unix_nanos": str(start)}
            (self.work / "network-expected.json").write_text(json.dumps(expected))
            base = self.start_web('web')
            subprocess.run(['node', str(Path(__file__).with_name('browser.mjs')), base,
                            str(self.work), self.args.playwright, self.args.chromium],
                           check=True, timeout=180)
            report.update(status="passed", trace_id=trace_id, action_id=action_id)
        except Exception as error:
            report["error"] = str(error)
            raise
        finally:
            server.close()
            if self.web:
                self.web.terminate()
                try:
                    self.web.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    self.web.kill()
                    self.web.wait()
            self.runtime.stop()
            report["server_requests"] = server.requests
            (self.work / "result.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--bin-dir', type=Path, default=REPO / 'target/release')
    parser.add_argument('--agent', type=Path, default=Path('/home/yzh/.cargo/bin/xiaoo'))
    parser.add_argument('--agent-timeout', type=int, default=600)
    parser.add_argument('--port', type=int, default=18480)
    parser.add_argument('--playwright', default='/tmp/cliproxy-endpoint-browser/node_modules/playwright/index.mjs')
    parser.add_argument('--chromium', default='/home/yzh/.cache/ms-playwright/chromium-1248/chrome-linux-arm64/chrome')
    NetworkAcceptance(parser.parse_args()).run()
