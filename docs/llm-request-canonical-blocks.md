# LLM Request Canonical Blocks

This document defines the L0 retention model for LLM request bodies. It is the source of truth for the destructive refactor from inline provider JSON to trace-local canonical request blocks.

## Goal

Agent requests grow by replaying most previous conversation state on every turn. Storing every `llm.request` body inline on the action repeats the same system prompts, tool schemas, historical messages, and tool results many times. AcTrail now stores reconstructable request content as a canonical skeleton plus trace-local reusable blocks.

The model optimizes for three properties:

- A request can be reconstructed as canonical provider JSON.
- Repeated request blocks within the same trace are stored once.
- The action row stays small enough for action-tree, export, and viewer queries.

This is a breaking storage and configuration change. Old SQLite schemas are not migrated. Old `request_content = full_provider_json` configs are invalid.

## Retention Semantics

`[semantic_retention.L0_llm_call].request_content` supports:

| Value | Meaning |
| --- | --- |
| `none` | Keep no request body content beyond transport/link metadata. |
| `shape` | Keep shape, size, hash, model, and transport metadata, but no reconstructable body. |
| `canonical_blocks` | Store reconstructable canonical provider JSON through manifest, refs, and trace-local blocks. |

`canonical_blocks` is L0 semantic content storage. With `content_owner = highest_consumed`, a request body consumed into L0 canonical blocks must not also be retained as HTTP body text or raw payload body. Lower layers keep summary, byte counts, transport metadata, and evidence references.

Trajectory identification uses the canonical block projection to compare LLM text-history prefixes without hashing the request a second time. The generated daemon configuration includes:

```toml
[semantic_retention.l0_llm_call]
websocket_max_connections_per_process = 8

[semantic_retention.l0_llm_call.assembly]
max_buffer_bytes = 8388608
max_segment_ranges = 8192

[semantic_retention.l0_llm_call.stream_classifier]
soft_sniff_max_bytes = 16384

[semantic_retention.l0_llm_call.projection_state]
max_pending_requests_per_stream = 256
max_pending_responses_per_stream = 64
max_action_versions_per_trace = 4096
max_pending_trajectory_actions_per_trace = 1024
max_tool_entries_per_trace = 4096

[semantic_retention.l0_llm_call.trajectory]
enabled = true
max_active_trajectories_per_scope = 128
max_candidate_nodes_per_trajectory = 256
max_prefix_nodes_per_scope = 65536
max_history_atoms_per_request = 4096
max_blocks_per_atom = 64
max_structural_bytes_per_atom = 4096
idle_ttl = "30m"

[semantic_retention.l2_http.exchange]
max_pending_requests_per_stream = 256
max_pending_responses_per_stream = 32
response_lateness = "1s"
```

All capacity values, `idle_ttl`, and `response_lateness` must be positive. Integer capacity values must also fit the host `usize`; invalid daemon configuration fails at startup. The settings bound daemon memory only and are not sent to the eBPF probe. `assembly.max_buffer_bytes` bounds incomplete plaintext retained by one HTTP/1 direction or one HTTP/2 connection direction and also bounds the combined text and tool-output bytes accumulated for one WebSocket response; `assembly.max_segment_ranges` bounds retained payload metadata. `stream_classifier.soft_sniff_max_bytes` bounds only the decoded SSE prefix used for provider guessing; reaching it keeps the stream undetermined and must not discard bytes, clear events, or permanently classify the stream as ordinary SSE. `projection_state` bounds correlation queues per stream and action-version, deferred-trajectory, and tool-correlation state per trace. Capacity exhaustion emits a projection diagnostic before evicting state. An evicted open request is exported as a partial call, an already-exported pending response remains available, and action-version eviction never suppresses the current action. Explicitly truncated or partial operations and assembly capacity overflow discard only the affected direction; projection stays quarantined until a trusted HTTP message boundary is observed. HTTP exchange reconciliation retains only the configured number of request and response identities per stream. A response action that is projected before its request action can be updated during `response_lateness` only when the captured outbound operation began no later than the response; expired or causally impossible responses remain unassociated. Request capacity exhaustion quarantines that stream rather than shifting later responses onto unknown requests. `websocket_max_connections_per_process` bounds concurrently tracked upgraded LLM connections, including main-agent and subagent Codex sessions; the oldest accepted connection that has not bound a business stream is evicted first at capacity, otherwise the least-recently-observed active connection is evicted. Trajectory identification is effectively enabled only when the LLM layer is enabled and `request_content = "canonical_blocks"`; selecting `none` or `shape` keeps request capture operational but disables trajectory assignment because reusable content hashes are unavailable.

Inference version 2 also recognizes provider-managed context chains. A request whose `previous_response_id` exactly matches an observed, successfully paired response inherits that response's request trajectory. The lookup is isolated by trace, process, and request classifier, is bounded by the existing trajectory candidate limits, and takes precedence over content-prefix matching. Because request and response projections from opposite directions may commit out of order, a delta request with a valid but not-yet-known reference is held within the same bound until the response ID is registered. If the reference remains unresolved for `idle_ttl`, at trace close, or capacity prevents deferral, the request becomes a new root. Delta requests are never indexed as complete history. Provider IDs remain runtime-only metadata; payload storage and the lineage schema are unchanged.

## Reconstruction Contract

AcTrail reconstructs canonical provider JSON, not the exact HTTP request bytes. The following are not preserved:

- Original whitespace.
- Original object key order.
- Non-semantic JSON formatting differences.

The canonicalization version is stored with each manifest. Version 2 rules:

- Parse request body as JSON.
- Sort object keys lexicographically.
- Preserve array order.
- Serialize without insignificant whitespace.
- Hash canonical UTF-8 bytes.

The reconstructed canonical body must hash to the manifest body hash. Missing blocks, hash mismatches, or malformed skeletons are read errors, not silent partial success.

Non-JSON request bodies do not use canonical blocks. They are retained as `shape` only: byte counts, JSON state, model when extractable, and hashes, without body text.

## Storage Model

Each `llm.request` action may have one manifest:

```text
llm_request_manifests
  manifest_id
  trace_id
  action_id
  format_version
  canonical_body_hash BLOB
  canonical_body_bytes
  skeleton_json
```

The skeleton is provider JSON with large reusable nodes replaced by ordinal placeholders:

```json
{"messages":[{"$actrail_llm_block":0},{"$actrail_llm_block":1}],"model":"deepseek-v4-flash"}
```

Each placeholder has one ref:

```text
llm_request_block_refs
  manifest_id
  ordinal
  block_id
```

The block table stores trace-local content-addressed canonical JSON bytes:

```text
llm_request_blocks
  block_id
  trace_id
  block_hash BLOB
  uncompressed_bytes
  encoded_bytes
```

The storage schema intentionally avoids repeating long text identifiers in refs. `action_id` is stored once in the manifest row, `block_hash` is stored once per unique block as a 32-byte BLOB, and each ref stores only integer ids plus the ordinal needed for reconstruction. `block_kind` is not stored because reconstruction does not use it.

Blocks are trace-local. AcTrail must not deduplicate LLM request blocks across traces, and public APIs must not expose cross-trace block equality as a feature.

## Block Boundaries

Version 2 uses non-overlapping nested block boundaries. The same splitter is used whether trajectory identification is enabled or disabled, so toggling trajectory inference cannot change physical request storage:

- Every top-level `tools[]` item is a block.
- A message without `content` is a block. Otherwise its envelope remains in the skeleton and its scalar or array `content` is split by content-item rules.
- String content and the `text` scalar of typed `text`, `input_text`, or `output_text` content use the scalar text value as the block. This gives equivalent string and typed-text payloads the same reusable block identity.
- A `tool_result`/`tool-result` item's `content` value is a block; all other item fields remain in the skeleton.
- Other content items may remain whole-item blocks.
- Top-level `input` message items use the same message/content splitter. Other top-level `input` values and `prompt` values remain blocks.

Nested splitting remains fully reconstructable: payload placeholders occur at their original object field or array position, while fields such as `cache_control` remain in the skeleton (or in an unchanged whole-item block). Hydration recursively replaces those placeholders and must reproduce the complete canonical provider JSON.

Trajectory atoms reuse the hashes produced by these storage blocks; version 2 does not add a second request or payload hash. Their structural descriptor combines the stable message-envelope whitelist (`role`, `name`, `tool_call_id`, and `type`) with ordered content descriptors. For known text and tool-result items, top-level `cache_control` is excluded because providers can add or move cache hints on replay without changing conversation history. Text descriptors exclude `text`; tool-result descriptors exclude `content`; their other stable fields are retained.

Unknown content item types remain whole-item blocks and their existing block hash participates in trajectory identity. Their fields, including `cache_control`, therefore use exact matching. This deliberately limits cache-insensitive normalization to understood item schemas, avoids copying arbitrarily large unknown items into structural descriptors, and prevents different unknown payloads from being connected speculatively.

The block hash is computed from canonical block JSON bytes. If a block row already exists for the same `(trace_id, block_hash)`, the stored bytes must match. Hash collision or canonicalization mismatch is fail-fast.

## Action Attributes

`semantic_actions.attributes` for `llm.request` must keep linking and provenance metadata required by live/runtime linking and exports:

- Model and byte counts.
- HTTP protocol, method, authority, path, and stream id when available.
- Payload stream key, operation id, sequence, source boundary, library, and symbol.
- Payload aggregate span metadata.
- Content state, manifest version, canonical body bytes, and canonical body hash for JSON block mode.
- Trajectory ID and trajectory inference version when trajectory identification is available.

It must not contain `llm.request.payload_text`.

The one full-body field it may carry is `llm.request.canonical_body_json`, and
only where an operator has enabled request body export. It is absent by
default. When body export is enabled, `llm.request.canonical_body_export_state`
is `exported` if that field is present or `too_large` if the canonical UTF-8
body exceeded `request_body_export_max_bytes` and was omitted entirely.

## Read, API, and Export Policy

Default action-tree, OTEL, and JSON export views do not inline reconstructed request bodies. They expose content state, sizes, model, transport metadata, and references.

OTEL egress of a request body requires all three explicit settings:

1. `semantic_retention.l0_llm_call.request_content = "canonical_blocks"`;
2. `semantic_retention.l0_llm_call.request_body_export = "canonical_json"`;
3. the OTEL plugin's `attribute_mode = "full"`.

`full` exports attributes already present on an action; it does not enable the
daemon's optional body attribute. Keeping these controls separate prevents an
upgrade from widening the egress boundary for deployments that already use
`full`.

Full request content is only returned by explicit content reads with a bounded size:

```text
GET /api/traces/{trace_id}/actions/{action_id}/content/llm-request?max_bytes=N
```

The response must distinguish:

- `available`
- `shape_only`
- `truncated`
- `unavailable`
- `corrupt`

Waterfall and action-detail views should use previews from the content API rather than reading inline action attributes.

## Purge and Privacy

Purge for a trace must delete manifests, refs, and blocks in the same retention path that deletes semantic actions. Since blocks are trace-local, purge never has to maintain cross-trace reference counts.

Hashes are not harmless. They can be checked against common prompts or tool schemas. Public API and export surfaces should avoid exposing block lists or block hashes by default. Internal hashes exist to verify reconstruction and trace-local deduplication.
