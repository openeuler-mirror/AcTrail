# AcTrail Regression Runner

This directory provides a tester-facing regression entrypoint. It orchestrates
existing AcTrail E2E scripts and writes both human-readable and machine-readable
reports.

```bash
uv venv --python /usr/bin/python3
source .venv/bin/activate
uv pip install -r tests/regression/requirements.txt
python3 tests/regression/test_all.py
```

`requirements.txt` installs the Python workload dependencies needed by the
regression cases, such as LangGraph. The default LangGraph workload uses HTTPS
and the eBPF OpenSSL shared-library attach path, so use a system-Python venv
whose `_ssl` shared object links dynamic OpenSSL.
If a LangGraph workload is changed to a plain HTTP API URL, the socket payload
path is valid and dynamic OpenSSL is not required for that case. The runner
framework itself only uses the Python standard library.

Useful variants:

```bash
python3 tests/regression/test_all.py --list
python3 tests/regression/test_all.py --list --suite payload
python3 tests/regression/test_all.py --suite agent
python3 tests/regression/test_all.py --case e2e-xiaoo
python3 tests/regression/test_all.py --case http-llm-projection
python3 tests/regression/test_all.py --suite full --strict
python3 tests/regression/test_all.py --output-dir /tmp/actrail-regression
```

Default behavior is quick and dependency-aware. Missing optional real agents or
API keys are reported as `SKIP`; actual failures in selected runnable cases are
reported as `FAIL`.

Checks use an `expected` / `found` detail shape when the pass condition depends
on observable facts. Multi-fact checks list each found fact as an unordered item
so reviewers can see which evidence made the check pass. Real LLM cases also
propagate `evidence.llm_request.*=` and `evidence.llm_response.*=` lines from
their E2E scripts into the Markdown/JSON reports. These lines include the
observed model, source boundary, route, payload byte count, and configured body
excerpts. They intentionally do not print raw HTTP headers because provider
headers may contain credentials. If no semantic response span exists, the report
says `evidence.llm_response=not exported` instead of claiming response-body
capture.
The real agent cases for Claude Code, opencode, xiaoO, and LangGraph require a
complete `llm.request` and `llm.response` semantic exchange and require both
action kinds in the OTEL export. The local `http-llm-projection` case remains a
request-projection check for the plain HTTP provider/proxy path.

The Claude Code case first runs a direct `claude -p` availability prompt and
requires the configured marker before starting AcTrail. The prompt, marker, and
timeout are configured by `availability_prompt`, `availability_marker`, and
`availability_timeout_seconds` in `tests/payload/claude-code/workload.conf`.
This checks the host's current Claude authentication and default model access
independently from the capture path. If the direct prompt fails, the case is
skipped as an external prerequisite failure instead of reported as an AcTrail
capture regression.

The xiaoO case follows the same pattern with a direct
`xiaoo run --no-tools --max-turns 1` availability prompt before AcTrail starts.
Its prompt, marker, and timeout are configured by `availability_prompt`,
`availability_marker`, `availability_timeout_seconds`, and
`availability_max_turns` in
`tests/agent-trace/xiaoo-rustls/workload.conf`. If xiaoO cannot reach its
configured provider or default model outside AcTrail, the case is skipped before
payload capture checks run.

The `docs-examples` case keeps the one-command regression aligned with
`docs/examples/TESTING.md`. It automates the documented quick-start attach
workflow, the local HTTP/2 payload workflow, the extended-observation attach
workflow, and the xiaoO -> Claude agent-invocation example. It also runs the
external HTTP/1.1 and HTTP/2 OpenAI-compatible docs paths when the configured
API key environment variable is present; otherwise those provider checks are
reported as `SKIP`. Each documented example's regression step lives under its
own `tests/regression/cases/09-docs-examples/case_*` directory; the root
`test.py` only wires the ordered scenario list. Its regression-only control
timeouts and viewer polling budgets, including xiaoO availability turn count,
live in `tests/regression/cases/09-docs-examples/workload.conf`.
The xiaoO -> Claude docs check validates the exported OTEL edge by matching the
`agent.invocation` parent and child pids to xiaoO and Claude `process.exec`
spans in the same trace; terminal completion markers are reported separately and
are not treated as proof of the invocation edge.

Automatic discovery and overrides:

- Claude Code: the payload case first supports Node/OpenSSL launchers. For
  native ELF entrypoints such as `claude.exe`, it then checks executable
  OpenSSL symbols and finally the configured Bun/static-BoringSSL symbol-map or
  byte-pattern discovery path. If none of those exposes an `SSL_write` attach
  point, the test can still pass through `Syscall/socket-syscall` when the
  configured Claude provider route is plain HTTP. Plain HTTP traces do not
  require inbound TLS response rows even when the installed Claude runtime is
  discoverable. The report includes concrete TLS discovery details so testers
  can tell which path was used. When the target
  host cannot export the native binary, run
  `python3 docs/preflight/claude_native_profile.py --json-output /tmp/actrail-claude-native-profile.json --symbol-map-output /tmp/actrail-claude-code-boringssl.map`
  on that host; it emits a text-only package/build-id/profile report. The
  regression renders the docs example 06 operator template, so the Claude case
  validates the same config surface used by `tests/payload/claude-code/`.
- opencode: scans `opencode` launchers on `PATH`, checks adjacent `.opencode`
  binaries and launcher binaries, and first tries the checked-in Bun/BoringSSL
  map. If the build-id changed, it detects BoringSSL offsets from the current
  binary and generates a temporary matching map. `OPENCODE_BIN_PATH` overrides
  discovery.
- xiaoO: scans `xiaoo` on `PATH` and runs the selected binary. The rustls case
  requires `tls-probe-point-finder fast` to resolve a complete rustls plaintext
  plan for the selected binary before launch; stripped x86_64 builds can pass
  through the checked rustls static patterns. Socket-only HTTP CONNECT evidence
  is not accepted as a substitute for this case. `XIAOO_BINARY` overrides
  discovery.
- xiaoO HTTP proxy: runs xiaoO with a generated config under
  `target/agent-trace/xiaoo-http-proxy/`. The generated config points xiaoO at a
  local plain HTTP OpenAI-compatible reverse provider shim from
  `tests/support/llm-http-proxy/`, and the shim forwards to the configured HTTPS
  upstream using `upstream_api_key_env` from
  `tests/agent-trace/xiaoo-http-proxy/workload.conf`. The manual fixed-port
  config lives at `tests/agent-trace/xiaoo-http-proxy/xiaoo-config.toml`. This
  case requires
  complete inbound and outbound `Syscall/socket-syscall` payload rows and a
  complete `llm.call` / `llm.request` / `llm.response` action graph.
- LangGraph: scans the current Python, active virtualenv, repository `.venv`,
  and Python executables on `PATH` for `langgraph`, `requests`, and dynamic
  OpenSSL when the configured API URL is HTTPS. Python builds with static or
  built-in `_ssl` are detected and rejected for the default HTTPS workload.
  `LANGGRAPH_PYTHON` overrides discovery.
- `--output-tail-chars`: amount of stdout/stderr retained per command in the
  generated reports. Use `0` when a full transcript is needed.
- `evidence_text_max_chars`: per-workload config key controlling the maximum
  request/response body characters printed in `evidence.llm_*` lines.
- External agent cases use their workload `launch_timeout_seconds`. If an
  auto-discovered optional agent times out before producing the expected marker,
  the default run reports `SKIP`; setting the corresponding binary override
  makes that same timeout fail-fast.
- `http-llm-projection`: local-only HTTP request with an OpenAI-style
  `model/messages` body. It verifies that `Syscall` socket payloads are
  projected into a pretty OTEL `llm.request` span, so it covers the plain HTTP
  provider/proxy path without external network or API keys.

If an override is set but invalid, the selected case fails fast. If no override
is set and discovery cannot find a runnable dependency, the case is reported as
`SKIP` with the scanned candidates in the report.

Status markers:

```text
[√] pass
[x] fail
[-] skip
[!] warn
```
