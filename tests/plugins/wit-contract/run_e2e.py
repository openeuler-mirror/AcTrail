#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
WIT = ROOT / "crates/core/plugin_system/wit/actrail-plugin.wit"


def require(needle: str, haystack: str) -> None:
    if needle not in haystack:
        raise RuntimeError(f"WIT contract missing required text: {needle}")


def reject(needle: str, haystack: str) -> None:
    if needle.lower() in haystack.lower():
        raise RuntimeError(f"WIT contract contains rejected concept: {needle}")


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
        "world managed-control-plugin",
        "read-payload: func(ref: payload-ref, offset: u64, max-bytes: u64)",
        "consume: func(batch: observation-batch)",
        "decide: func(request: decision-request)",
        "record actor-process-identity",
        "actor-process-identity: actor-process-identity",
        "context-ref: option<string>",
        "record decision-summary",
        "record file-policy-view",
        "record file-policy-apply-request",
        "record file-policy-list-filter",
        "record file-policy-list-result",
        "record file-policy-match-dry-run-request",
        "record file-policy-match-dry-run-result",
        "record plugin-command-request",
        "record plugin-command-result",
        "enum file-policy-apply-status",
        "query-context: func(context-ref: string, query: string) -> result<decision-summary, string>",
        "file-access-current-match-get: func(context-ref: string, query: string) -> result<file-policy-view, string>",
        "file-policy-rules-list: func(filter: file-policy-list-filter, cursor: option<string>, limit: u32) -> result<file-policy-list-result, string>",
        "file-policy-rules-match-dry-run: func(request: file-policy-match-dry-run-request) -> result<file-policy-match-dry-run-result, string>",
        "file-policy-rules-apply: func(request: file-policy-apply-request) -> result<file-policy-apply-result, string>",
        "interface management-command",
        "export management-command",
        "handle-command: func(request: plugin-command-request) -> result<plugin-command-result, string>",
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

    print(f"wit_contract={WIT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
