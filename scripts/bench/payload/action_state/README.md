# Action lifecycle acceptance

Run a real xiaoo session against the local HTTPS MaaS. The model requests one
repository-owned stdio MCP tool and returns a final response. The probe checks
the actual JSON-RPC request, execution marker and successful response.

```bash
python3 -m scripts.bench.payload.action_state.run \
  --out local/bench/payload/action-state --modes P C \
  --bin-dir target/release --config-dir scripts/bench/payload/configs
```

Each mode refreshes the default operator configuration and applies its mode file
plus isolated runtime paths. Only its own daemon is stopped. Existing release
binaries are used; no global installer or build runs. This is functional
validation, not a CPU benchmark.

`--modes bare` verifies the real agent/MCP fixture without AcTrail. The default
xiaoo tool name is `mcp__state_probe__emit_marker`; `--model-tool-name` is an
explicit override. The usual bash-only tool selection is removed.

P/C validation uses the selected viewer, independent lifecycle/classification
and evidence tables, and stored canonical MCP contents according to the resolved
retention configuration. It requires five terminal MCP actions, observed
relationships, server command MCP classification, and request and response
evidence when payload retention is enabled. Final-state queries do not establish
a full update history.

MCP timeout, error, truncation and online OTLP cases are not exercised.

Read completed traces through an independently owned web process:

```bash
python3 -m scripts.bench.payload.action_state.web \
  --run-dir local/bench/payload/action-state --bin-dir target/release \
  --out local/bench/payload/action-state-web
```

This checks action detail attributes/evidence against the selected viewer output,
root metadata, and complete tree traversal with one child per page. Relation
checks report observed endpoint identities and hydrated references; absent
relation types are explicitly unexercised. The web process uses an ephemeral
localhost port and exits after verification. The source daemon is not started.

`python3 -m scripts.bench.payload.action_state.relations --graphs <actions.json> ...
--out <report.json>` checks the same relation identities in other real-agent
viewer exports, including OpenCode tool results.
