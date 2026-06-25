# Agent Invocation E2E

This case verifies real silent agent invocation discovery driven by LLM evidence:

- The outer agent is configured by `agent_command` in `workload.conf`.
- The reusable prompt is rendered from `agent_prompt.template`.
- The child Claude prompt, timeout, and sentinel are configured in `workload.conf`.
- `claude_extra_args` enables Claude's Bash tool so the default workload covers a real Claude child command.
- `drain_attempts` and `drain_sleep_seconds` bound how long the runner waits for async semantic projection to export the child Claude command evidence.

Run it after building release binaries:

```bash
cargo build --release
python3 tests/process/agent-invocation/run_e2e.py
```

Use the reusable probes for diagnosis instead of rewriting the prompt on the shell:

```bash
python3 tests/process/agent-invocation/run_probe.py bare-xiaoo
python3 tests/process/agent-invocation/run_probe.py direct-claude
python3 tests/process/agent-invocation/run_probe.py strace-xiaoo
```

Expected result:

- `actrailctl launch` output contains `ACTRAIL_AGENT_TREE_OK` returned by the foreground `claude -p` command.
- The exported pretty OTEL JSON is written to `/tmp/actrail-agent-invocation-e2e.otlp.json`.
- OTEL contains a `process.exec` span for `claude` with `seccomp_observed=true` and argv containing `claude -p`.
- OTEL contains an `llm.request` span for the same Claude process.
- OTEL contains a successful Bash `command.invocation` span whose direct parent is the same Claude process.
- OTEL contains an `agent.invocation` span whose child command line contains `claude`; its parent is Claude's direct launcher, which may be a shell or timeout wrapper rather than the outer xiaoO process.

This case requires root, `/root/projects/xiaoO/target/release/xiaoo`, a working default xiaoO config, a working `claude` CLI, and external network access. It is not a mock and should fail fast if those prerequisites are missing.
