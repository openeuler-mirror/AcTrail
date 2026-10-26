# Storage delivery failure acceptance

```bash
python3 -m scripts.bench.payload.storage_delivery.run \
  --bin-dir target/release \
  --output-dir local/bench/payload/storage-delivery
```

Each scenario starts an independently owned daemon with refreshed default
configuration, a real xiaoo agent, the repository local HTTPS MaaS, and an online
OTLP/HTTP receiver. Four real requests and three shell tool executions must
complete. SQLite triggers reject event writes or AgentIdentity writes while
online LLM actions and trace finalization remain observable.

The AgentIdentity scenario sets descendant observation depth to zero and checks
the living agent's generation and depth in the map owned by its daemon. It
requires `bpftool` and privileges to inspect that map. The tool commands wait
briefly to permit this observation.

The selected release directory must contain actraild, actrailctl, actrailweb,
and their collector assets. No build or global installation runs. Cleanup stops
only this run's processes and preserves the database, fault SQL, effective
configuration, agent/MaaS logs, and online OTLP documents under the output path.

To verify trace-close persistence failure during normal daemon shutdown:

```bash
python3 -m scripts.bench.payload.storage_delivery.finalization \
  --bin-dir target/release \
  --output-dir local/bench/payload/storage-delivery-finalization
```

The real xiaoo agent invokes a local MCP tool. The tool records its actual
execution and waits without responding. Once the pending MCP action is stored,
the fixture shuts down its daemon normally. A SQLite trigger rejects the
`semantic_action_state` update with `finalization_reason=1` (TraceClosed).
Acceptance requires that exact trigger in the daemon log, an online MCP action
with error/partial terminal state, the unchanged pending database state, and a
successful daemon shutdown. Only the fixture's own process group is terminated
afterward. This scenario verifies shutdown finalization; the event and identity
scenarios separately verify terminal trace completion.

`--verify-existing` checks saved real workload artifacts and writes
`verification.json`, preserving the original `acceptance.json`. Metadata-only
OTLP omits the trace-close attribute; the trigger predicate and failure log
establish the finalization reason.

TODO: process-exit MCP cleanup removes pending calls in
`McpProjector::clear_protocol_state` before trace finalization. Normal process
exit therefore does not exercise the pending MCP TraceClosed update; this
separate lifecycle issue requires its own authorized fix.

The event case verifies analysis during ongoing event-write failures; it does
not establish that a particular collector poll contained both event and payload.
