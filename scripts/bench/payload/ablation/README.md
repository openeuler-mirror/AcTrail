# CPU ablation

Ordinary pipe/FIFO and Unix socket observation is disabled in every stage.

```sh
sudo -E python3 -m scripts.bench.payload.ablation.run --agent-bin <AGENT_BIN>
```

| Setting | Default |
|---|---|
| Suite | All stages in `suite.toml` |
| Agent / backend | xiaoo / NoOp |
| Agent executable | Explicit `--agent-bin`; no PATH lookup |
| Workload | 120 HTTPS requests, 119 tools, 128 input bytes, TPOT 3 ms |
| Samples | 1 warmup + 3 measured runs per stage; serial |
| Binaries | Existing `target/release` |
| Configuration | Fresh daemon defaults + base + each stage's full patch |
| CPU | Task + daemon through trace completion |
| Extra CPU | Task + daemon − bare |
| Extra percentage | (Task + daemon − bare) / bare × 100% |
| Percentage denominator | Same workload's measured bare CPU |
| Output | `local/bench/ablation/<timestamp>/report.md`, `run.log`, `measurements/results.json` |
| Socket paths | Owned short alias under `/tmp`; removed on success, retained on failure |

| Stages | Parent |
|---|---|
| P → no-startup → no-socket → no-stdio → no-fs → no-resource → no-tls → no-net → lifecycle-only → no-semantics | Previous stage |
| ipc-context-only | no-tls |
| mcp-off | P |

| Requirement | Fields / status |
|---|---|
| TLS discovery controls | `payload.tls.direct_startup_discovery_enabled`, `payload.tls.direct_dynamic_discovery_enabled` |
| Semantic control | `semantic_retention.projection_enabled` |
| ipc-context-only | TODO: no external switch for the historical context-only bypass |

| Operation | Command |
|---|---|
| Selected stages | `sudo -E python3 -m scripts.bench.payload.ablation.run --agent-bin <AGENT_BIN> --only P no-tls no-net` |
| Short full-suite verification | `sudo -E python3 -m scripts.bench.payload.ablation.run --agent-bin <AGENT_BIN> --warmups 0 --rounds 1 --agent-turns 2` |
| Custom modes | `sudo -E python3 -m scripts.bench.payload.ablation.run --agent-bin <AGENT_BIN> --base <BASE_PATCH> --case baseline=<PATCH> --case variant=<PATCH>` |
| Other binary / agent | `sudo -E python3 -m scripts.bench.payload.ablation.run --bin-dir <RELEASE_DIR> --agent-kind opencode --agent-bin <AGENT>` |
| Existing report | `python3 -m scripts.bench.payload.ablation.run --report-only <OUTPUT_DIR>` |

## P storage ablation

```sh
sudo -E python3 -m scripts.bench.payload.ablation.run --agent-bin <AGENT_BIN> --suite scripts/bench/payload/ablation/storage.toml
```

This suite measures bare, P with NoOp, and P with SQLite. It initializes fresh daemon defaults and applies the P configuration: TLS uses `bpf-copy` with 65535-byte segment/operation limits; MCP, stdio and enforcement are disabled; LLM identity and timing are retained, while LLM content and raw payload retention are disabled. Startup TLS discovery is enabled; daemon-side dynamic TLS discovery is disabled. The two observed stages differ only in storage backend, its associated default parameters and isolated runtime paths. Every SQLite sample must contain all workload LLM requests and responses.

Each stage can set `storage.backend`; stages without a selection use NoOp. A suite may supply its own `base`, which `--base` overrides. Reports include task CPU, daemon CPU, command elapsed time and drain time. CPU measurement ends at the same trace finalization log entry for both backends, after synchronous transaction commits. Daemon initialization and shutdown/checkpoint costs are outside this window. SQLite evidence checks run after measurement and verify trace health and stored LLM request/response counts.

## P storage encoding ablation

Use `--suite scripts/bench/payload/ablation/storage-encoding.toml` to compare NoOp, default SQLite, cold-field compression disabled, event payload dictionary disabled, and both disabled. Collection settings and capture limits are identical. Each encoding option preserves the stored data; evaluate CPU and database size separately. Long request histories can exceed the fixed 65535-byte TLS capture limit: request/response coverage does not imply complete request bodies. Retain and check `capture_limited` evidence.

| Artifact | Contents |
|---|---|
| `suite.toml` | Stage patch, comparison parent and incremental change |
| `cases/` | One complete patch per stage |
| `report.md` | CPU values, percentages, parent deltas and stage status; updated per sample |
| `inputs/`, `measurements/configs/` | Input and effective configurations |
| `measurements/results.json` | Per-sample CPU, actual workload counts, binary paths and source state |
