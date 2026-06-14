# Python LangGraph Agent

This example launches a real LangGraph Python workload with `actrailctl launch`.
The LangGraph node uses LangChain's `ChatOpenAI` integration to call an
OpenAI-compatible chat completions endpoint. There is no local stub, replay
server, HTTP relay, framework instrumentation, TLS-library path rewrite, or
protocol downgrade for AcTrail.

Defaults target DeepSeek:

- API endpoint: `https://api.deepseek.com/chat/completions`
- Model: `deepseek-chat`
- Key environment variable: `DEEPSEEK_API_KEY`

## Files

| File | Purpose |
| --- | --- |
| `operator.conf` | AcTrail operator config with process/network capture, TLS plaintext payload, socket payload, HTTP/1.x and HTTP/2 analyzers, payload text export, and semantic action export. |
| `workload.conf` | Provider, prompt, timeout, drain, and OTEL output settings. |
| `workload.py` | LangGraph `StateGraph` workload whose node performs the real LLM request through `langchain-openai`. |
| `run_e2e.py` | End-to-end runner and assertions for payloads, semantic actions, and OTEL export without adapting the workload to AcTrail. |

## Preconditions

- Run from the repository root in a Linux/WSL root shell.
- Build release binaries first: `cargo build --release`.
- Set `DEEPSEEK_API_KEY`.
- Use a Python interpreter with `langgraph` and `langchain-openai`. If the
  default Python lacks those packages, set `LANGGRAPH_PYTHON`.

```bash
python3 -m venv /tmp/actrail-langgraph-system-venv
uv pip install --python /tmp/actrail-langgraph-system-venv/bin/python langgraph langchain-openai
```

This case intentionally uses the framework's normal HTTPS client path. If the
real LLM call succeeds but AcTrail cannot capture and project the request, keep
the failure as evidence of a Python framework-agent capability gap instead of
changing the workload to fit AcTrail's current capture path.

## Provider Overrides

The runner accepts these environment overrides:

```bash
ACTRAIL_LLM_BASE_URL=https://api.deepseek.com
ACTRAIL_LLM_CHAT_PATH=/chat/completions
ACTRAIL_LLM_MODEL=deepseek-chat
ACTRAIL_LLM_API_KEY_ENV=DEEPSEEK_API_KEY
ACTRAIL_LLM_PROMPT='Reply exactly with ACTRAIL_LANGGRAPH_DOCS_OK'
```

`ACTRAIL_LLM_CHAT_PATH` must end with `/chat/completions`, because
`ChatOpenAI` configures the API base URL and appends that chat-completions route
internally.

If you override the prompt and still want a strict answer marker check, also set
`ACTRAIL_LLM_EXPECTED_OUTPUT_FRAGMENT`. Without that extra variable, the runner
only requires a non-empty real LLM answer.

## Run

```bash
python3 docs/examples/clean.py --example python-langgraph-agent
LANGGRAPH_PYTHON=/tmp/actrail-langgraph-system-venv/bin/python \
  python3 docs/examples/09.python-langgraph-agent/run_e2e.py
```

Expected result:

- The workload prints a non-empty `llm_answer_json=...` line and
  `ACTRAIL_LANGGRAPH_AGENT_COMPLETE`.
- `actrailviewer payloads` shows a complete successful outbound plaintext row
  from the runtime TLS path for HTTPS providers, or from `Syscall/socket-syscall`
  for plain HTTP provider routes.
- `actrailviewer actions` contains a complete successful `llm.request`.
- `export-otel` writes `/tmp/actrail-python-langgraph-agent.otlp.json` with an
  `actrail.action.kind=llm.request` span containing the configured model and
  prompt text.

For HTTPS runs, a socket row that only contains `CONNECT <host>:443` is proxy
tunnel evidence, not chat request body capture.

If the workload reaches the real LLM but these AcTrail assertions fail, the
case has done its job: it identifies a missing capture/projection capability for
the normal Python LangGraph/LangChain agent path.

`llm.response` OTEL evidence is printed when present, but it is not a required
pass condition for this docs transfer test.
