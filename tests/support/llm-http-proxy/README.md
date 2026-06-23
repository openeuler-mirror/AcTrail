# LLM HTTP Proxy Test Support

`provider_proxy.py` is a local OpenAI-compatible provider shim for manual and automated agent tests. By default it listens on `http://127.0.0.1:18098`, reads the upstream credential from `DEEPSEEK_API_KEY`, and forwards requests to `https://api.deepseek.com` with `Authorization: Bearer`.

Manual start:

```bash
export DEEPSEEK_API_KEY=...
python3 tests/support/llm-http-proxy/provider_proxy.py
```

For deterministic local streaming coverage, run with `--mode local-stream`. That mode does not call an upstream provider. It emits an OpenAI-compatible `text/event-stream` response whose terminal JSON chunk carries `finish_reason="stop"` and intentionally omits a `data: [DONE]` sentinel, matching endpoints that close streams this way.

Every default can be overridden through CLI flags. Automated E2E cases pass their values from their workload config, so they can use a random local port without changing the manual fixed-port config.
