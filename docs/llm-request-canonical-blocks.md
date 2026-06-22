# LLM Request Canonical Blocks

This document defines the L0 retention model for LLM request bodies. It is the source of truth for the destructive refactor from inline provider JSON to trace-local canonical request blocks.

## Goal

Agent requests grow by replaying most previous conversation state on every turn. Storing every `llm.request` as inline `llm.request.body_json` repeats the same system prompts, tool schemas, historical messages, and tool results many times. AcTrail now stores reconstructable request content as a canonical skeleton plus trace-local reusable blocks.

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

## Reconstruction Contract

AcTrail reconstructs canonical provider JSON, not the exact HTTP request bytes. The following are not preserved:

- Original whitespace.
- Original object key order.
- Non-semantic JSON formatting differences.

The canonicalization version is stored with each manifest. Version 1 rules:

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

Version 1 uses non-overlapping block boundaries:

- Every top-level `tools[]` item is a block.
- Every top-level `messages[]` item is a block unless the message `content` is an array.
- For a message whose `content` is an array, the message envelope remains in the skeleton and each `content[]` item is a block.
- Top-level `prompt` and `input` values are blocks.

The block hash is computed from canonical block JSON bytes. If a block row already exists for the same `(trace_id, block_hash)`, the stored bytes must match. Hash collision or canonicalization mismatch is fail-fast.

## Action Attributes

`semantic_actions.attributes` for `llm.request` must keep linking and provenance metadata required by live/runtime linking and exports:

- Model and byte counts.
- HTTP protocol, method, authority, path, and stream id when available.
- Payload stream key, operation id, sequence, source boundary, library, and symbol.
- Payload aggregate span metadata.
- Content state, manifest version, canonical body bytes, and canonical body hash for JSON block mode.

It must not contain full request body fields:

- `llm.request.body_json`
- `llm.request.body_text`
- `llm.request.payload_text`

## Read, API, and Export Policy

Default action-tree, OTEL, and JSON export views do not inline reconstructed request bodies. They expose content state, sizes, model, transport metadata, and references.

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
