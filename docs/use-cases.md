# AcTrail Use Cases

This document maps common operator questions to the AcTrail feature path and the example that proves it.

## Use Case Matrix

| Question | Use This Path | Validate With |
| --- | --- | --- |
| What process tree did the agent create? | eBPF process lifecycle plus viewer process/events output. | [Example 01](examples/01.quick-start/README.md), [Example 03](examples/03.extended-observation-e2e/README.md) |
| Did one agent silently launch another agent? | `actrailctl launch` plus process seccomp exec context and `agent.invocation` semantic action. | [Example 07](examples/07.xiaoo-claude-agent-invocation/README.md) |
| What LLM request and response did Claude Code exchange? | Executable TLS payload capture plus HTTP/2 and `llm.request` semantic assembly. | [Example 06](examples/06.claude-code-tls-capture/README.md) |
| What LLM request and response did opencode exchange? | Bun/static-BoringSSL executable TLS payload capture plus proxy-aware HTTP semantics. | `python3 tests/agent-trace/run_case.py opencode-bun` |
| What outbound LLM request did a Rust/rustls agent send? | Executable rustls symbol-map TLS capture. | [xiaoO rustls guide](llm-capture/xiaoo-rustls/README.md) |
| What outbound LLM request did a LangGraph Python agent send? | Dynamic OpenSSL shared-library TLS payload capture around an OpenAI-compatible HTTPS call. | `python3 tests/agent-trace/run_case.py langgraph-openai` |
| What HTTP request/response did a non-TLS local service exchange? | Socket plaintext payload with HTTP sniffing and HTTP/1.x application analyzer. | [Example 05](examples/05.http-payload-unified/README.md) |
| Did file enforcement actually block the target? | Fanotify permission backend plus client-side stdout and Enforcement events. | [Example 04](examples/04.fanotify-enforcement-e2e/README.md) |
| What files, IPC, stdio, and resources did a workload touch? | Extended eBPF observation, stdio payload, provider labels, resource metrics. | [Example 03](examples/03.extended-observation-e2e/README.md) |
| Can I export higher-level actions to observability tooling? | `actrailviewer export-otel` over semantic actions. | [Example 04](examples/04.fanotify-enforcement-e2e/README.md), [Example 06](examples/06.claude-code-tls-capture/README.md), [Example 07](examples/07.xiaoo-claude-agent-invocation/README.md) |
| What is the runtime overhead? | `tests/performance/run_benchmark.py` task-runtime benchmark. | [Performance README](../tests/performance/README.md) |

## Agent Invocation Discovery

Goal: detect a tree-shaped call such as one agent launching `claude -p ...`.

Use:

```bash
python3 docs/examples/07.xiaoo-claude-agent-invocation/run_e2e.py
```

Expected evidence:

- The monitored workload prints `ACTRAIL_AGENT_TREE_OK`.
- OTEL export contains a `process.exec` span for `claude -p`.
- OTEL export contains an `agent.invocation` span linking the parent agent executable to the child agent command.

This use case does not require TLS payload capture. It proves process-level semantic discovery.

## Complete LLM Request Capture

Goal: retain the LLM request and response bytes, and derive an `llm.request` action from the request.

Use the capture path that matches the target runtime:

| Target Runtime | Capture Path |
| --- | --- |
| Dynamic OpenSSL | `payload_tls_source = shared-library`, `payload_tls_resolver = openssl-symbols`. |
| Node/OpenSSL executable | `payload_tls_source = executable`, `payload_tls_resolver = openssl-symbols`. |
| Bun/static-BoringSSL | `payload_tls_resolver = boringssl-static` for built-in x86_64/aarch64 related-entry detection, or `bun-static-boringssl` with a matching build-id symbol map containing `SSL_read` and `SSL_write`. |
| Rust/rustls | `payload_tls_resolver = rustls-symbol-map` with a matching build-id symbol map from the binary or debuginfo. |
| Plain HTTP endpoint/proxy | `payload_socket_enabled = true`, `payload_socket_capture_backend = bpf-copy-seccomp-fallback`, plus HTTP/1.x application parsing. |

Real agent acceptance cases:

| Case | Command | Runtime Boundary |
| --- | --- | --- |
| Claude Code | `python3 tests/agent-trace/run_case.py claude-code` | HTTPS/TLS when supported, otherwise plain HTTP socket plaintext if configured by provider/proxy. |
| opencode | `python3 tests/agent-trace/run_case.py opencode-bun` | Bun/static-BoringSSL TLS writes or plain HTTP socket plaintext; model pinned to `deepseek/deepseek-chat`. |
| xiaoO | `python3 tests/agent-trace/run_case.py xiaoo-rustls` | Rust/rustls plaintext write functions or plain HTTP socket plaintext. Stripped HTTPS/HTTP CONNECT binaries need matching debuginfo/TLS symbols before this payload case can pass. |
| LangGraph | `LANGGRAPH_PYTHON=/path/to/python python3 tests/agent-trace/run_case.py langgraph-openai` | Python dynamic OpenSSL shared-library writes for HTTPS; socket plaintext for HTTP API URLs. |

On hosts that require a local proxy for external network access, keep the proxy environment active for these commands. Do not assume the provider route is HTTPS: some agent configs use a plain HTTP base URL or proxy. When HTTPS is used, proxy tunnel facts such as `CONNECT api.deepseek.com:443` may appear alongside decrypted TLS request rows. When plain HTTP is used, the request payload source is `Syscall/socket-syscall`.

For large LLM requests, prefer `bpf-copy-seccomp-fallback` on the relevant payload path: eBPF directly copies operations no larger than the stable inline budget, while seccomp notification remains installed before exec and is used as the user-space read fallback for larger operations. The current real-agent examples use a 4MB user-read operation budget for both TLS and plain HTTP, avoiding fixed multi-MB ringbuf records while still keeping normal LLM requests complete.

Run through `actrailctl launch`; do not use `track-add` for TLS backends that require pre-exec seccomp setup (`seccomp-user-read` and `bpf-copy-seccomp-fallback`).

Expected evidence:

- `actrailviewer payloads` shows complete outbound plaintext rows: `TlsUserSpace` for HTTPS/TLS or `Syscall/socket-syscall` for plain HTTP.
- Operation state is `success`.
- Captured size equals original size for the operation being validated.
- `actrailviewer actions` contains a complete successful `llm.request`.
- `actrailviewer export-json` contains payload nodes if payload export is enabled.
- `actrailviewer export-otel` contains an `llm.request` span.

Rows marked `Truncated` or operation state `partial` mean the configured capture budget is not sufficient for that request.

## HTTP Semantics

Goal: inspect application protocol facts instead of only raw bytes.

Use:

- Plain HTTP: [Example 05](examples/05.http-payload-unified/README.md).
- HTTPS/2: [Example 02](examples/02.llm-http-payload-capture/README.md) or [Example 06](examples/06.claude-code-tls-capture/README.md).

Expected evidence:

- Payload rows show the plaintext source boundary: `Syscall` for plain HTTP, `TlsUserSpace` for HTTPS.
- `Application` events show HTTP request/response, HTTP/2 frames, or HTTP/2 DATA facts depending on the protocol.
- `llm.request` actions are assembled from retained outbound request payloads when the payload is complete.

HTTP parsing consumes retained plaintext payloads. It should not depend on whether bytes came from OpenSSL, BoringSSL, rustls, or socket syscalls.

## File Enforcement

Goal: prove the observed system both enforced a policy and recorded the decision.

Use:

```bash
python3 docs/examples/04.fanotify-enforcement-e2e/run_e2e.py \
  --otel-output /tmp/actrail-fanotify-e2e.otlp.json
```

Expected evidence:

- Target-side stdout includes `allowed=ok`.
- Target-side stdout includes `denied=permission_denied`.
- Viewer output contains one allow and one deny Enforcement event.
- OTEL export contains two `enforcement.decision` spans when `--otel-output` is used.

If the target prints `denied=unexpected_success`, the policy did not block access even if AcTrail produced events.

## Broad Local Observation

Goal: validate process, file, IPC, network, stdio, provider, and resource observation on one host.

Use:

```bash
python3 docs/examples/clean.py --example extended-observation
./target/release/ebpf_probe verify-live \
  --config docs/examples/03.extended-observation-e2e/observation.conf
```

Expected evidence includes:

```text
process_events=exec,exit,fork,signal
file_events=...
net_events=...
ipc_events=...
resource_events=process_tree
stdio_payloads=stderr:outbound,stdin:inbound,stdout:outbound
```

The fork event comes from `sched/sched_process_fork`; a host does not need `syscalls/sys_enter_fork` for this path.

## OpenTelemetry Export

Goal: feed higher-level AcTrail actions into an observability pipeline.

Use:

```bash
./target/release/actrailviewer export-otel \
  --config <operator.conf> \
  --trace-id <TRACE_ID> \
  --output /tmp/actrail-trace.otlp.json
```

Expected evidence:

- Top-level JSON contains `resourceSpans`.
- Semantic spans contain `actrail.action.kind`.
- Action kinds currently covered by examples include `process.exec`, `agent.invocation`, `http.message`, `llm.request`, and `enforcement.decision`.

Not every raw fact is currently an OTEL span. Validate low-level network rows, IPC rows, resource samples, provider labels, and payload rows through viewer commands or JSON graph export when no semantic span exists.

## Performance Measurement

Goal: measure task runtime overhead, not one-time wrapper setup time.

Use:

```bash
python3 tests/performance/run_benchmark.py \
  --case agent \
  --mode baseline,observed-ebpf-core,observed-ebpf-payload,observed-seccomp-agent \
  --repetitions 10 \
  --output local/performance-agent.md
```

Read `task_runtime_ms` for the primary overhead question. `outer_wall_ms` is reported separately to show runner and `actrailctl launch` control-plane cost. The report uses distribution-level evidence rather than treating same-numbered iterations as paired samples.
