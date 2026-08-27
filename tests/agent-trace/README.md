# Agent Trace E2E Cases

These cases verify agent runtime traces through compiled AcTrail binaries.

Run from the repository root after `cargo build --release`:

```bash
python3 tests/agent-trace/run_case.py xiaoo-rustls
python3 tests/agent-trace/run_case.py agentscope-openai
python3 tests/agent-trace/run_case.py gnutls-nss-llm
sudo python3 tests/agent-trace/multi-container-xiaoo/run_e2e.py
```

Use `python3 tests/agent-trace/run_case.py all` to run every case registered by the shared runner.

`xiaoo-rustls` requires a working xiaoO binary and provider credentials. It records the `tls-probe-point-finder fast` result and validates the actual provider route. HTTPS routes require complete `TlsUserSpace` request payload evidence; plain HTTP routes require complete `Syscall/socket-syscall` request payload evidence.

`agentscope-openai` runs AgentScope 2.x against the local OpenAI-compatible HTTP shim in `tests/support/llm-http-proxy/`. Its default local-stream mode requires no external API key or network.

`gnutls-nss-llm` builds controlled locator programs and test libraries for GnuTLS and NSS/NSPR. It verifies payload capture and the resulting HTTP and LLM semantic actions.

`multi-container-xiaoo` runs two real xiaoO processes in separate Docker containers against one host daemon. It verifies that their trace identities, actions, and request markers remain isolated.

Expected proof:

- Payload cases show complete outbound plaintext payload rows.
- Full exchange cases contain successful `llm.request` and `llm.response` actions.
- Full exchange cases export `llm.request` and `llm.response` OTEL spans.
