# Hidden Agent Invocation E2E

This case verifies that agent identity is driven by outbound LLM evidence, not by command names alone.

Process shape:

```text
agent_a
  -> bash script_b.sh
       -> xiaoo run -p ...
```

`agent_a` is compiled from `agent_a.c` during the test. It uses OpenSSL directly so launch-time TLS probe resolution has deterministic hook points for the root process.

Expected semantic evidence:

- `agent_a` performs a real DeepSeek HTTPS request and its `process.exec` action is marked `agent.identity.status=observed`.
- `xiaoo` performs its own real LLM request and its `process.exec` action is marked `agent.identity.status=observed`.
- Only the direct `script_b.sh` launcher edge is upgraded to `agent.invocation`; there must not be an ancestor `agent_a -> xiaoo` invocation.

Run after building release binaries:

```bash
cargo build --release
python3 tests/process/hidden-agent-invocation/run_e2e.py
```

The test requires `DEEPSEEK_API_KEY`, `xiaoo` on `PATH`, `gcc` with OpenSSL headers/libraries, root privileges, and external network access.
