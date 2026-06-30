# Agent Trace E2E Cases

These cases verify real agent runtime traces through compiled AcTrail binaries. They are not mock unit tests.

Run from the repository root after `cargo build --release`:

```bash
python3 tests/agent-trace/run_case.py claude-code
python3 tests/agent-trace/run_case.py opencode-bun
python3 tests/agent-trace/run_case.py xiaoo-rustls
python3 tests/agent-trace/run_case.py xiaoo-http-proxy
python3 tests/agent-trace/run_case.py agentscope-openai
python3 tests/agent-trace/run_case.py langgraph-openai
python3 tests/agent-trace/run_case.py go-net-http
```

`claude-code`, `opencode-bun`, and `xiaoo-rustls` require the corresponding real CLI or binary and working provider credentials in the environment. These agents may reach the LLM provider through HTTPS/TLS or through a plain HTTP endpoint/proxy; the E2E cases accept either complete `TlsUserSpace` payload rows or complete `Syscall/socket-syscall` plaintext rows. `langgraph-openai` requires a Python interpreter with `langgraph` and `requests`, plus `DEEPSEEK_API_KEY`; the default workload uses DeepSeek's OpenAI-compatible HTTPS API to exercise a real LangGraph-based Python agent without adding framework instrumentation.

`agentscope-openai` runs a native AgentScope 2.x `Agent` with `OpenAIChatModel` against the local OpenAI-compatible HTTP shim in `tests/support/llm-http-proxy/`. The default `local-stream` mode requires no external API key or network. Set `AGENTSCOPE_PYTHON=/path/to/python` when the current interpreter does not already have `agentscope` and `openai` installed. The pass condition requires complete inbound and outbound `Syscall/socket-syscall` payload rows, complete successful `llm.call`, `llm.request`, and `llm.response` actions, linked LLM graph evidence, Web action-tree reachability, and OTEL spans for request and response.

`xiaoo-http-proxy` is the real-agent plain HTTP coverage. Its automatic runner starts the generic OpenAI-compatible provider shim from `tests/support/llm-http-proxy/`, writes a temporary xiaoO config under `target/agent-trace/xiaoo-http-proxy/`, and launches xiaoO against a local plain HTTP endpoint. The default workload uses `proxy_mode = local-stream`, so no upstream API key is required: the shim emits deterministic OpenAI-compatible SSE with a final JSON `finish_reason` chunk and no `data: [DONE]` marker. Set `proxy_mode = forward` to forward to the configured HTTPS upstream provider using `DEEPSEEK_API_KEY`, while xiaoO only receives a dummy local API key. The checked-in `xiaoo-http-proxy/xiaoo-config.toml` is for manual runs and defaults to `http://127.0.0.1:18098`; the proxy script uses the same address when started without arguments. The pass condition requires complete inbound and outbound `Syscall/socket-syscall` payload rows plus complete successful `llm.call`, `llm.request`, and `llm.response` actions, with no failed `llm.response` rows. This is not an `HTTP_PROXY`/CONNECT test; CONNECT would not expose the LLM request body to socket plaintext capture.

Manual xiaoO HTTP proxy smoke path, using separate terminals for the long-running proxy and daemon commands:

```bash
python3 tests/support/llm-http-proxy/provider_proxy.py --mode local-stream
```

```bash
target/release/actrailctl --config tests/agent-trace/xiaoo-http-proxy/operator.conf clean
target/release/actraild --config tests/agent-trace/xiaoo-http-proxy/operator.conf run
```

```bash
export ACTRAIL_XIAOO_HTTP_PROXY_API_KEY=actrail-local-proxy-key
target/release/actrailctl --config tests/agent-trace/xiaoo-http-proxy/operator.conf \
  launch --name agent-xiaoo-http-proxy -- \
  xiaoo --cli run --config tests/agent-trace/xiaoo-http-proxy/xiaoo-config.toml \
    --no-tools --max-turns 1 --prompt '请只输出 ACTRAIL_XIAOO_HTTP_PROXY_OK，不要解释'
```

For `xiaoo-rustls`, the runner requires `tls-probe-point-finder fast` to return a complete rustls plan for the selected xiaoO binary. Stripped x86_64 builds are supported through the checked rustls static patterns when both `rustls_buffer_plaintext` and `rustls_take_received_plaintext` are found. A socket-only HTTP CONNECT trace is not accepted for this case because it does not prove HTTPS request-body plaintext capture.

The `opencode-bun` case is pinned in `opencode-bun/workload.conf` to `deepseek/deepseek-chat`. This keeps the case independent from the local opencode default model and avoids stale provider keys. In a proxy-only network, keep the shell's local proxy environment active. The case validates `tls-probe-point-finder fast --provider auto --source auto` before launch; the resolved operator config keeps `payload_tls_source`, `payload_tls_resolver`, and `payload_tls_library` set to `auto`.

For `langgraph-openai`, use a Python build whose `_ssl` extension links dynamic OpenSSL. Python imports `_ssl` lazily, so this case uses the eBPF OpenSSL shared-library attach path instead of the launch-time `tls-sync` inline hook. The system Python venv path works in this environment:

```bash
python3 -m venv /tmp/actrail-langgraph-system-venv
uv pip install --python /tmp/actrail-langgraph-system-venv/bin/python langgraph requests
LANGGRAPH_PYTHON=/tmp/actrail-langgraph-system-venv/bin/python \
  python3 tests/agent-trace/run_case.py langgraph-openai
```

For `go-net-http`, the runner builds multiple Go workloads under `tests/agent-trace/go-net-http/workloads/` and uses a generic full-monitor configuration. The test must not specify `go-pclntab`, `payload_tls_library = go`, or a Go binary path in the config. AcTrail resolves Go TLS probe points automatically when the child process execs. The default runtime path validates standard-library `net/http` and a small Go library wrapper. A cgo/OpenSSL workload is also built and checked with the automatic fast probe resolver; set `ACTRAIL_GO_OPENSSL_E2E=1` to run its provider traffic in an environment where the OpenSSL workload can reach the LLM endpoint.

Expected proof:

- payload cases show complete outbound plaintext payload rows. HTTPS/TLS requests use `TlsUserSpace`; plain HTTP requests use `Syscall/socket-syscall`.
- full exchange cases contain complete successful `llm.request` and `llm.response` semantic actions.
- full exchange cases export `llm.request` and `llm.response` OTEL spans.
- process invocation cases additionally validate `process.exec` / `agent.invocation` spans in their own scripts.
