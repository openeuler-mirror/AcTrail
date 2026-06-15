# AcTrail Usage Guide

This guide covers the normal operator workflow. For transfer-test case details, use [examples/TESTING.md](examples/TESTING.md).

## 1. Build

```bash
cargo build --release
```

Expected binaries:

```text
target/release/actraild
target/release/actrailctl
target/release/actrailviewer
target/release/actrailweb
target/release/ebpf_probe
```

## 2. Check The Host

Run the read-only preflight first:

```bash
python3 docs/preflight/platform_preflight.py --color always
```

For a new transfer-test host, run the local smoke suite:

```bash
python3 docs/preflight/platform_preflight.py --run-smoke --color always
```

A red `[required]` row blocks the corresponding live example. Optional agent executable rows only block agent-specific cases such as Claude Code or opencode.

## 3. Choose A Config

Use an example config for the workflow you are validating:

| Workflow | Config |
| --- | --- |
| Existing-process process/network observation | `docs/examples/01.quick-start/operator.conf` |
| Broad process/file/network/IPC/resource/stdout observation | `docs/examples/03.extended-observation-e2e/operator.conf` |
| Plain HTTP socket payload and HTTP/1.x semantics | `docs/examples/05.http-payload-unified/operator.conf` |
| Local HTTPS/2 TLS payload and HTTP/2 semantics | `docs/examples/02.llm-http-payload-capture/http2-local/operator.conf` |
| Full real-agent monitor validation | `docs/examples/08.full-monitor-validation/operator.conf` |
| xiaoO outbound LLM request capture | `docs/examples/06.xiaoo-tls-capture/operator.conf` |
| xiaoO launching Claude Code process tree discovery | `docs/examples/07.xiaoo-claude-agent-invocation/operator.conf` |

For manual real-agent validation, start with `docs/examples/08.full-monitor-validation/`.
For acceptance across multiple runtimes, use the E2E suite under `tests/agent-trace/`. The suite renders its own case configs and validates viewer output plus OTEL spans:

```bash
python3 tests/agent-trace/run_case.py claude-code
python3 tests/agent-trace/run_case.py opencode-bun
python3 tests/agent-trace/run_case.py xiaoo-rustls
python3 tests/agent-trace/run_case.py langgraph-openai
```

`opencode-bun` is pinned to `deepseek/deepseek-chat` in `tests/agent-trace/opencode-bun/workload.conf`; keep the shell's local proxy environment active on proxy-only hosts. If the installed opencode build-id does not match the checked-in Bun/BoringSSL map, the case detects the current `SSL_read` and `SSL_write` offsets from configured byte patterns and writes a temporary matching map. `langgraph-openai` requires `langgraph`, `requests`, `DEEPSEEK_API_KEY`, and a Python build whose `_ssl` module links dynamic OpenSSL.

Initialize the default full-collection operator config:

```bash
sudo ./target/release/actraild init
```

The default path is `/etc/actrail/actraild.conf`. `actrailctl init` performs the same initialization. If the file already exists, `init` loads and validates it, reports success or the validation error, and exits without rewriting it. For a local test config, pass `--output local/operator.conf` or `--config local/operator.conf`.

Every runtime constant is explicit in the config. The generated default enables broad collection, but leaves blocking/enforcement disabled. Its socket plaintext fallback listens to `write`, `writev`, `sendto`, and `sendmsg`, so plain HTTP request bodies sent through vectored socket writes can produce `llm.request` evidence.

## 4. Clean Local Runtime Artifacts

Before rerunning an example:

```bash
./target/release/actrailctl clean --config <operator.conf>
```

For docs examples, prefer the helper because it also removes documented workload files:

```bash
python3 docs/examples/clean.py --example <example-name>
```

`clean` only removes artifacts declared by the config or example metadata. It is meant to replace repeated manual `/tmp/actrail-*` deletion.
When `[export] enabled = true`, enabled `otel-jsonl` route output files are also cleaned.

## 5. Start And Check The Daemon

```bash
./target/release/actraild --config <operator.conf> start
./target/release/actraild --config <operator.conf> status
./target/release/actrailctl doctor --config <operator.conf>
```

When `--config` is omitted, `actraild` and `actrailctl` load `/etc/actrail/actraild.conf`. If that file is missing or invalid, they fail with the config path and validation/read error.

Use foreground mode when running under a supervisor:

```bash
./target/release/actraild --config <operator.conf> run
```

`actraild start` writes logs to the config's `log_path`. `actrailctl doctor` verifies control-plane readiness; it does not prove that every configured collector has already observed target activity.

## 6. Attach Or Launch

Attach an already-running process:

```bash
./target/release/actrailctl track-add \
  --config <operator.conf> \
  --pid <PID> \
  --name <trace-name>
```

Launch a child under AcTrail:

```bash
./target/release/actrailctl launch \
  --config <operator.conf> \
  --name <trace-name> \
  -- \
  <command> <args>
```

Use `launch` for `tls-sync` TLS capture and process seccomp agent-invocation observation. Use `track-add` only when observing an already-running process is sufficient. The sync TLS runtime, event socket, and probe plan must be prepared before the target `exec`. Set `payload_tls_java_agent_enabled = true` only for Java JSSE HTTPS capture; the default is `false`.

## 7. Inspect A Trace

List traces:

```bash
./target/release/actrailctl list-traces --config <operator.conf>
./target/release/actrailviewer traces --config <operator.conf>
```

Inspect a trace:

```bash
./target/release/actrailviewer summary --config <operator.conf> --trace-id <TRACE_ID>
./target/release/actrailviewer processes --config <operator.conf> --trace-id <TRACE_ID>
./target/release/actrailviewer events --config <operator.conf> --trace-id <TRACE_ID> --head 80
./target/release/actrailviewer network --config <operator.conf> --trace-id <TRACE_ID> --head 40
./target/release/actrailviewer payloads --config <operator.conf> --trace-id <TRACE_ID> --head 40
./target/release/actrailviewer actions --config <operator.conf> --trace-id <TRACE_ID>
./target/release/actrailviewer diagnostics --config <operator.conf> --trace-id <TRACE_ID>
```

Read one retained payload:

```bash
./target/release/actrailviewer payload \
  --config <operator.conf> \
  --trace-id <TRACE_ID> \
  --segment-id <SEGMENT_ID> \
  --format text
```

Use `--format hex` when the payload is not valid UTF-8.

## 8. Export

Export graph JSON:

```bash
./target/release/actrailviewer export-json \
  --config <operator.conf> \
  --trace-id <TRACE_ID> \
  --output /tmp/actrail-trace.json
```

Export OpenTelemetry OTLP JSON:

```bash
./target/release/actrailviewer export-otel \
  --config <operator.conf> \
  --trace-id <TRACE_ID> \
  --output /tmp/actrail-trace.otlp.json
```

Payload bytes/text appear in JSON export only when the operator config enables `export_payload_bytes_enabled` or `export_payload_text_enabled`. Offline OTEL export emits semantic action spans at export time. For live span streaming, enable:

```conf
[export]
enabled = true

[[export.routes]]
name = "live-otel"
kind = "otel-jsonl"
delivery = "best-effort"
enabled = true

[export.routes.otel-jsonl.live-otel]
path = "/tmp/actrail-live-spans.otlp.jsonl"
overwrite_enabled = false
queue_capacity = 1024
flush_every_spans = 1
```

The live file is compact JSONL: one OTLP JSON document per line, one span per document. Queue-full drops are reported as `RuntimeDropped` diagnostics; writer I/O errors fail the daemon instead of silently falling back.

## 9. Real Agent Acceptance

After `cargo build --release`, run the real agent trace suite when validating runtime-specific LLM capture:

```bash
LANGGRAPH_PYTHON=/path/to/dynamic-openssl-python \
  python3 tests/agent-trace/run_case.py all
```

Use the `LANGGRAPH_PYTHON` override only for the LangGraph case when the default Python does not have `langgraph` and `requests`, or when its `_ssl` module is statically linked. On the current WSL validation host, a system-Python venv was used:

```bash
python3 -m venv /tmp/actrail-langgraph-system-venv
/tmp/actrail-langgraph-system-venv/bin/python -m pip install langgraph requests
LANGGRAPH_PYTHON=/tmp/actrail-langgraph-system-venv/bin/python \
  python3 tests/agent-trace/run_case.py langgraph-openai
```

Expected proof for these cases is:

```text
payload rows: outbound TlsUserSpace, success, captured size equals original size
actions: complete successful llm.request, plus llm.response when inbound response payload is retained
OTEL export: spans with actrail.action.kind=llm.request, and llm.response when that action was assembled
```

The opencode case also shows proxy tunnel facts such as `CONNECT api.deepseek.com:443` when the host must reach the network through the local proxy.

## 10. Web UI

Start the read-only Web UI from an operator config:

```bash
./target/release/actrailweb --config <operator.conf>
```

Override address or port for a local run:

```bash
./target/release/actrailweb --config <operator.conf> --addr 127.0.0.1 --port 18080
```

`actrailweb` reads storage; it does not start collection by itself.

The current UI is centered on the semantic action graph: the left rail selects a trace, the main canvas renders the agent process and recursively expanded action/evidence swimlanes, and the right panel shows rows, payload text, attributes, and raw JSON for the selected node. It reads low-level trace snapshots for counts/details and `/api/traces/<TRACE_ID>/action-tree` for stored semantic action links; the browser does not invent semantic relationships that were not emitted by the observation pipeline.

## 11. Stop

```bash
./target/release/actraild --config <operator.conf> stop
```

If the trace was attached to a long-running process and should stop before process exit:

```bash
./target/release/actrailctl track-remove --config <operator.conf> --trace-id <TRACE_ID>
```

## Common Failure Signals

| Symptom | Meaning |
| --- | --- |
| `tracefs mount is missing` or `perf_event_open` failure | eBPF tracepoint/uprobe attachment is blocked by the host. See [platform-requirements.md](platform-requirements.md). |
| `BootstrapGap` diagnostic on manual attach | The process existed before AcTrail attached, so pre-attach history may be incomplete. |
| TLS payload rows missing in a `tls-sync` case | The target was not launched through `actrailctl launch`, the sync runtime was not loaded, the probe plan did not match the target binary, or sync payload events were not consumed. |
| Payload operation `partial` or `Truncated` | The configured per-operation or per-segment capture budget was too small for that payload. |
| OTEL export has no expected span | The semantic action was not assembled for that trace, or that low-level fact has no OTEL span mapping yet. |
