# Agent Trace E2E Cases

These cases verify real agent runtime traces through compiled AcTrail binaries. They are not mock unit tests.

Run from the repository root after `cargo build --release`:

```bash
python3 tests/agent-trace/run_case.py claude-code
python3 tests/agent-trace/run_case.py opencode-bun
python3 tests/agent-trace/run_case.py xiaoo-rustls
python3 tests/agent-trace/run_case.py langgraph-openai
```

`claude-code`, `opencode-bun`, and `xiaoo-rustls` require the corresponding real CLI or binary and working provider credentials in the environment. These agents may reach the LLM provider through HTTPS/TLS or through a plain HTTP endpoint/proxy; the E2E cases accept either complete `TlsUserSpace` payload rows or complete `Syscall/socket-syscall` plaintext rows. `langgraph-openai` requires a Python interpreter with `langgraph` and `requests`, plus `DEEPSEEK_API_KEY`; the default workload uses DeepSeek's OpenAI-compatible HTTPS API to exercise a real LangGraph-based Python agent without adding framework instrumentation.

For `xiaoo-rustls`, a stripped release binary can still be runnable as an
agent but not observable at the HTTPS plaintext boundary unless matching rustls
`PlaintextSink` symbols are available from the binary or debuginfo. If that
binary reaches its provider through HTTP CONNECT, socket capture can prove the
proxy tunnel but cannot decode the encrypted request body; configure xiaoO for a
plain HTTP provider route or provide debuginfo/TLS symbols for this payload
case.

The `opencode-bun` case is pinned in `opencode-bun/workload.conf` to
`deepseek/deepseek-chat`. This keeps the case independent from the local
opencode default model and avoids stale provider keys. In a proxy-only network,
keep the shell's local proxy environment active. If the checked-in
Bun/BoringSSL map does not match the installed opencode build-id, the case uses
the byte patterns in `workload.conf` to detect the current `SSL_write` offset
and writes a temporary matching map.

For `langgraph-openai`, use a Python build whose `_ssl` extension links dynamic
OpenSSL. The system Python venv path works in this environment:

```bash
python3 -m venv /tmp/actrail-langgraph-system-venv
uv pip install --python /tmp/actrail-langgraph-system-venv/bin/python langgraph requests
LANGGRAPH_PYTHON=/tmp/actrail-langgraph-system-venv/bin/python \
  python3 tests/agent-trace/run_case.py langgraph-openai
```

Expected proof:

- payload cases show complete outbound plaintext payload rows. HTTPS/TLS requests use `TlsUserSpace`; plain HTTP requests use `Syscall/socket-syscall`.
- semantic actions contain a complete successful `llm.request`.
- OTEL export contains an `llm.request` span.
- process invocation cases additionally validate `process.exec` / `agent.invocation` spans in their own scripts.
