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


def record_body(name: str, wit: str) -> str:
    marker = f"record {name} {{"
    start = wit.find(marker)
    if start < 0:
        raise RuntimeError(f"WIT contract missing record: {name}")
    end = wit.find("\n  }", start)
    if end < 0:
        raise RuntimeError(f"WIT contract record is not closed: {name}")
    return wit[start:end]


def main() -> int:
    if not WIT.exists():
        raise RuntimeError(f"WIT contract file missing: {WIT}")
    raw = WIT.read_text(encoding="utf-8")

    for required in [
        "package actrail:plugin@0.2.0;",
        "interface types",
        "interface host",
        "interface observation-context-read",
        "interface trace-analysis-read",
        "interface trace-file-state-read",
        "interface alert-write",
        "interface observation-consumer",
        "interface post-trace-analyzer",
        "interface control-decider",
        "world observation-plugin",
        "world post-trace-observation-plugin",
        "world control-plugin",
        "world managed-control-plugin",
        "read-payload: func(ref: payload-ref, offset: u64, max-bytes: u64)",
        "consume: func(batch: observation-batch)",
        "analyze: func(task: post-trace-task)",
        "semantic-actions-list: func(offset: option<u64>, limit: u32)",
        "get: func(action-id: string) -> result<trace-file-state, string>",
        "submit: func(request: alert-write-request) -> result<_, string>",
        "decide: func(request: decision-request)",
        "record file-change-record",
        "change-kind: file-change-kind",
        "lifecycle-transition: option<trace-lifecycle-state>",
        "record alert-draft",
        "record alert-write-request",
        "alert-token: option<list<u8>>",
        "definition-key: string",
        "payload-json: string",
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

    semantic_action = record_body("semantic-action-record", raw)
    for duplicated in ["trace-id:", "summary:"]:
        reject(duplicated, semantic_action)

    alert_draft = record_body("alert-draft", raw)
    for duplicated in [
        "plugin",
        "config",
        "evidence",
        "action-id",
        "title",
        "severity",
        "kind:",
    ]:
        reject(duplicated, alert_draft)

    print(f"wit_contract={WIT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
