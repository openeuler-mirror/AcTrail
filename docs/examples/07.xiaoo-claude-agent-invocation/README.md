# xiaoO Invokes Claude Code

This example verifies process-level agent invocation discovery when a real xiaoO agent silently launches Claude Code with `claude -p ...`.

It is a process/semantic trace case, not an LLM payload capture case. `payload_tls_enabled = false` and `payload_socket_enabled = false` on purpose. The expected proof is in the exported OpenTelemetry trace: a seccomp-observed `process.exec` span for `claude -p ...` and an `agent.invocation` span whose parent is xiaoO and whose child is Claude Code.

## Files

| File | Purpose |
| --- | --- |
| `operator.conf` | AcTrail operator config for process exec seccomp observation and agent invocation semantics. |
| `workload.conf` | Workload values for the real xiaoO -> Claude Code run. |
| `agent_prompt.template` | Prompt template that instructs xiaoO to run exactly one foreground `claude -p ...` command. |
| `run_e2e.py` | Thin docs entrypoint that runs the shared real E2E runner with this example's config. |

## Preconditions

- Run from the repository root as root.
- Build release binaries first: `cargo build --release`.
- `xiaoo` is on `PATH`; `which xiaoo` should print the executable that will be launched. Set `agent_command` in `workload.conf` only when testing a specific non-PATH binary.
- `claude` is on `PATH` and can answer `claude -p ...` with the current shell authentication.
- External network access is available for Claude Code.

Do not edit `/tmp` paths by hand. Use the cleanup helper or the E2E script's built-in `actrailctl clean` step.

## Run

```bash
python3 docs/examples/clean.py --example xiaoo-claude
python3 docs/examples/07.xiaoo-claude-agent-invocation/run_e2e.py
```

Expected terminal output includes:

```text
ACTRAIL_AGENT_TREE_OK
agent_invocation_trace_id=1
otel_output=/tmp/actrail-xiaoo-claude-agent-invocation.otlp.json
agent invocation e2e complete
```

The script fails fast if xiaoO, Claude Code, AcTrail binaries, seccomp/eBPF privileges, or the expected OTEL spans are missing.

## Manual Inspection

After a successful run, inspect the exported OTLP JSON:

```bash
jq '[.resourceSpans[].scopeSpans[].spans[] | select(any(.attributes[]?; .key=="actrail.action.kind" and .value.stringValue=="agent.invocation"))] | length' \
  /tmp/actrail-xiaoo-claude-agent-invocation.otlp.json

jq '[.resourceSpans[].scopeSpans[].spans[] | select(any(.attributes[]?; .key=="seccomp_observed" and .value.stringValue=="true"))] | length' \
  /tmp/actrail-xiaoo-claude-agent-invocation.otlp.json
```

The first command must print at least `1`. The second command must be nonzero. The agent invocation span attributes should contain `agent.parent.executable` ending in `xiaoo` and `agent.child.command_line` containing `claude -p`.

## What This Proves

- `actrailctl launch` keeps AcTrail as the trace root.
- The launched xiaoO process inherits the trace.
- Claude Code is observed through launch-time `execve`/`execveat` seccomp notify.
- AcTrail assembles the low-level process facts into a higher-level `agent.invocation` semantic action.

This example does not assert Claude's natural-language response and does not capture Claude LLM request bytes. xiaoO and Claude Code may reach providers through HTTPS/TLS or plain HTTP; use `docs/examples/06.claude-code-tls-capture/` for full outbound Claude Code LLM request payload capture and `tests/agent-trace/xiaoo-rustls/` for xiaoO request-payload capture with either TLS or socket evidence.
