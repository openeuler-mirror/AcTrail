#!/usr/bin/env python3
from __future__ import annotations

import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
WIT = ROOT / "crates/core/plugin_system/wit/actrail-plugin.wit"


def require(needle: str, haystack: str) -> None:
    if needle not in haystack:
        raise RuntimeError(f"WIT contract missing required text: {needle}")


def reject(needle: str, haystack: str) -> None:
    if needle.lower() in haystack.lower():
        raise RuntimeError(f"WIT contract contains rejected concept: {needle}")


def run_contract_parser_check() -> None:
    command = [
        "cargo",
        "test",
        "--release",
        "-p",
        "plugin_system",
        "--test",
        "wit_contract",
    ]
    result = subprocess.run(
        command,
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    if result.returncode != 0:
        raise RuntimeError(
            "WIT parser contract check failed\n"
            f"command={' '.join(command)}\n"
            f"{result.stdout}"
        )


def main() -> int:
    if not WIT.exists():
        raise RuntimeError(f"WIT contract file missing: {WIT}")
    raw = WIT.read_text(encoding="utf-8")

    for required in [
        "package actrail:plugin@0.1.0;",
        "interface types",
        "interface host",
        "interface observation-consumer",
        "interface control-decider",
        "world observation-plugin",
        "world control-plugin",
        "read-payload: func(ref: payload-ref, offset: u64, max-bytes: u64)",
        "consume: func(batch: observation-batch)",
        "decide: func(request: decision-request)",
        "record actor-process-identity",
        "actor-process-identity: actor-process-identity",
        "context-ref: option<string>",
        "record decision-summary",
        "record file-policy-view",
        "record file-policy-update",
        "enum file-policy-write-status",
        "query-context: func(context-ref: string, query: string) -> result<decision-summary, string>",
        "file-policy-read: func(context-ref: string, query: string) -> result<file-policy-view, string>",
        "file-policy-write: func(context-ref: string, update: file-policy-update) -> result<file-policy-write-status, string>",
        "read-config: func(offset: u64, max-bytes: u64)",
        "record config-chunk",
    ]:
        require(required, raw)

    for rejected in [
        "ReplayBatch",
        "replay-batch",
        "out-process",
        "external-process",
        "stdio",
        "grpc",
        "timestamp-unix-nanos",
    ]:
        reject(rejected, raw)

    run_contract_parser_check()

    print(f"wit_contract={WIT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
