#!/usr/bin/env python3
"""Deterministic local workloads for AcTrail performance benchmarks."""

from __future__ import annotations

import argparse
import configparser
import http.client
import json
import mmap
import os
import shlex
import shutil
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


def main() -> int:
    args = parse_args()
    config = read_config(args.config)
    start_ns = time.perf_counter_ns()
    if args.case == "file":
        details = run_file(config)
    elif args.case == "process":
        details = run_process(config)
    elif args.case == "http":
        details = run_http(config, "http")
    elif args.case == "agent":
        details = run_agent(config)
    elif args.case == "claude-code":
        details = run_external_llm(config, "claude-code")
    elif args.case == "opencode":
        details = run_external_llm(config, "opencode")
    else:
        raise RuntimeError(f"unknown workload case {args.case}")
    elapsed_ms = (time.perf_counter_ns() - start_ns) / 1_000_000
    details["elapsed_ms_inside_workload"] = round(elapsed_ms, 3)
    print("BENCHMARK_RESULT " + json.dumps(details, sort_keys=True), flush=True)
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", required=True)
    parser.add_argument("--config", required=True)
    return parser.parse_args()


def read_config(path: str) -> configparser.ConfigParser:
    config = configparser.ConfigParser()
    loaded = config.read(path, encoding="utf-8")
    if not loaded:
        raise RuntimeError(f"benchmark config not found: {path}")
    return config


def run_file(config: configparser.ConfigParser) -> dict[str, object]:
    section = config["file"]
    workspace = clean_workspace(config, section["workspace"])
    operations = section.getint("operations")
    block = deterministic_bytes(section.getint("block_bytes"))
    mmap_bytes = section.getint("mmap_bytes")
    mmap_payload = deterministic_bytes(mmap_bytes)
    for index in range(operations):
        path = workspace / f"file-{index}.dat"
        renamed = workspace / f"file-{index}.renamed"
        path.write_bytes(block)
        if path.read_bytes() != block:
            raise RuntimeError(f"file read mismatch at operation {index}")
        with path.open("ab") as handle:
            handle.write(block)
        path.rename(renamed)
        with renamed.open("r+b") as handle:
            handle.truncate(len(block))
        renamed.unlink()
    mmap_path = workspace / "mapped.dat"
    mmap_path.write_bytes(b"\0" * mmap_bytes)
    with mmap_path.open("r+b") as handle:
        with mmap.mmap(handle.fileno(), mmap_bytes, access=mmap.ACCESS_WRITE) as mapped:
            mapped[:mmap_bytes] = mmap_payload
            mapped.flush()
    if mmap_path.read_bytes() != mmap_payload:
        raise RuntimeError("mmap payload mismatch")
    return {
        "case": "file",
        "file_operations": operations,
        "block_bytes": len(block),
        "mmap_bytes": mmap_bytes,
    }


def run_process(config: configparser.ConfigParser) -> dict[str, object]:
    section = config["process"]
    executions = section.getint("executions")
    helper_argv = shlex.split(section["helper_argv"])
    if not helper_argv:
        raise RuntimeError("process.helper_argv must not be empty")
    for _ in range(executions):
        result = subprocess.run(
            helper_argv,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        if result.returncode != 0:
            raise RuntimeError(f"helper process failed with exit={result.returncode}")
    return {
        "case": "process",
        "executions": executions,
        "helper_argv": helper_argv,
    }


def run_http(config: configparser.ConfigParser, case_name: str) -> dict[str, object]:
    section = config["http"]
    requests = section.getint("requests")
    body = deterministic_bytes(section.getint("body_bytes"))
    response = deterministic_bytes(section.getint("response_bytes"))
    request_path = section["path"]
    timeout_seconds = section.getfloat("timeout_seconds")
    expected_status = section.getint("expected_status")
    server = BenchmarkHttpServer(
        section["bind_host"],
        section.getint("bind_port"),
        expected_status,
        response,
    )
    server.start()
    try:
        for _ in range(requests):
            status, payload = post_once(server.host, server.port, request_path, body, timeout_seconds)
            if status != expected_status or payload != response:
                raise RuntimeError(f"unexpected HTTP result status={status}")
    finally:
        server.stop()
    return {
        "case": case_name,
        "http_requests": requests,
        "http_body_bytes": len(body),
        "http_response_bytes": len(response),
    }


def run_agent(config: configparser.ConfigParser) -> dict[str, object]:
    section = config["agent"]
    workspace = clean_workspace(config, section["workspace"])
    rounds = section.getint("rounds")
    file_payload = deterministic_bytes(section.getint("file_bytes"))
    http_body = deterministic_bytes(section.getint("http_body_bytes"))
    process_every = section.getint("process_every")
    if process_every <= 0:
        raise RuntimeError("agent.process_every must be positive")
    helper_argv = shlex.split(config["process"]["helper_argv"])
    request_path = section["http_path"]
    timeout_seconds = section.getfloat("http_timeout_seconds")
    expected_status = section.getint("http_expected_status")
    server = BenchmarkHttpServer(
        section["bind_host"],
        section.getint("bind_port"),
        expected_status,
        deterministic_bytes(len(http_body)),
    )
    server.start()
    process_runs = 0
    try:
        for index in range(rounds):
            path = workspace / f"agent-round-{index}.dat"
            path.write_bytes(file_payload)
            if path.read_bytes() != file_payload:
                raise RuntimeError(f"agent file mismatch at round {index}")
            if index % process_every == 0:
                result = subprocess.run(
                    helper_argv,
                    stdin=subprocess.DEVNULL,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    check=False,
                )
                if result.returncode != 0:
                    raise RuntimeError(f"agent helper failed with exit={result.returncode}")
                process_runs += 1
            status, _ = post_once(server.host, server.port, request_path, http_body, timeout_seconds)
            if status != expected_status:
                raise RuntimeError(f"agent HTTP status={status}")
            path.unlink()
    finally:
        server.stop()
    return {
        "case": "agent",
        "rounds": rounds,
        "file_bytes": len(file_payload),
        "http_body_bytes": len(http_body),
        "process_runs": process_runs,
    }


def run_external_llm(config: configparser.ConfigParser, section_name: str) -> dict[str, object]:
    section = config[section_name]
    command = shlex.split(section["command_argv"])
    if not command:
        raise RuntimeError(f"{section_name}.command_argv must not be empty")
    result = subprocess.run(
        command,
        stdin=subprocess.DEVNULL,
        text=True,
        capture_output=True,
        timeout=section.getfloat("timeout_seconds"),
        check=False,
    )
    combined_output = (result.stdout or "") + (result.stderr or "")
    if result.returncode != 0:
        raise RuntimeError(
            f"{section_name} command failed with exit={result.returncode}: "
            f"{combined_output[-1000:]}"
        )
    min_output = section.getint("min_combined_output_bytes")
    if len(combined_output.encode("utf-8")) < min_output:
        raise RuntimeError(f"{section_name} command produced less than {min_output} bytes")
    return {
        "case": section_name,
        "command": command[0],
        "stdout_bytes": len((result.stdout or "").encode("utf-8")),
        "stderr_bytes": len((result.stderr or "").encode("utf-8")),
    }


def clean_workspace(config: configparser.ConfigParser, raw_path: str) -> Path:
    path = Path(raw_path)
    safe_prefix = config["runner"]["safe_tmp_prefix"]
    if not path.is_absolute() or not str(path).startswith(safe_prefix):
        raise RuntimeError(f"refusing to clean unsafe benchmark workspace: {path}")
    if path.exists():
        shutil.rmtree(path)
    path.mkdir(parents=True)
    return path


def deterministic_bytes(size: int) -> bytes:
    if size < 0:
        raise RuntimeError(f"byte size must be non-negative: {size}")
    pattern = b"actrail-performance-benchmark\n"
    repeats, remainder = divmod(size, len(pattern))
    return pattern * repeats + pattern[:remainder]


def post_once(
    host: str,
    port: int,
    path: str,
    body: bytes,
    timeout_seconds: float,
) -> tuple[int, bytes]:
    connection = http.client.HTTPConnection(host, port, timeout=timeout_seconds)
    try:
        connection.request("POST", path, body=body)
        response = connection.getresponse()
        payload = response.read()
        return response.status, payload
    finally:
        connection.close()


class BenchmarkHttpServer:
    def __init__(self, host: str, port: int, status: int, response: bytes) -> None:
        self._response = response
        handler = self._make_handler(status, response)
        self._server = ThreadingHTTPServer((host, port), handler)
        self._thread = threading.Thread(target=self._server.serve_forever)
        self._thread.daemon = True

    @property
    def host(self) -> str:
        return str(self._server.server_address[0])

    @property
    def port(self) -> int:
        return int(self._server.server_address[1])

    def start(self) -> None:
        self._thread.start()

    def stop(self) -> None:
        self._server.shutdown()
        self._thread.join()
        self._server.server_close()

    @staticmethod
    def _make_handler(status: int, response_body: bytes) -> type[BaseHTTPRequestHandler]:
        class Handler(BaseHTTPRequestHandler):
            def do_POST(self) -> None:
                length = int(self.headers.get("Content-Length", "0"))
                _ = self.rfile.read(length)
                self.send_response(status)
                self.send_header("Content-Type", "application/octet-stream")
                self.send_header("Content-Length", str(len(response_body)))
                self.end_headers()
                self.wfile.write(response_body)

            def log_message(self, format: str, *args: object) -> None:
                return

        return Handler


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"benchmark workload failed: {error}", file=sys.stderr)
        raise SystemExit(1)
