# AcTrail Examples Transfer Test Checklist

This file is for QA handoff. Run commands from the repository root unless a step says otherwise.

## Global Preconditions

- Run in a Linux/WSL root shell.
- Before transfer testing on a new host, run the platform preflight:

```bash
python3 docs/preflight/platform_preflight.py --run-smoke --color always
```

The output is intentionally checklist-like: green `✓` is pass, red `✗` is fail, and yellow `!` means the runtime proof was skipped or needs attention. Do not mark an example passed on a host that failed a relevant `[required]` preflight row. `docs/platform-requirements.md` explains the architecture, BTF, tracefs, eBPF, seccomp/pidfd, fanotify, and agent executable TLS checks.
- Build release binaries first:

```bash
cargo build --release
```

- eBPF examples need a kernel that allows AcTrail to attach tracepoints/uprobes. On the WSL test host used during development, the live examples were run with:

```bash
sysctl kernel.perf_event_paranoid=-1
```

Restore the host policy after testing:

```bash
sysctl kernel.perf_event_paranoid=2
```

- Use the docs cleanup helper before each example. It calls `actrailctl clean --config` for operator configs and removes only documented `/tmp/actrail-*` workload artifacts parsed from example configs:

```bash
python3 docs/examples/clean.py --example <example-name>
```

- Do not hand-edit `/tmp` paths unless the document explicitly asks you to create an isolated copy of a config. Examples are expected to fail fast on stale sockets, stale pid files, unsupported kernel features, missing API keys, or missing `claude`.

## One-Command Regression

For fast regression before or during transfer testing, use the regression runner:

```bash
uv venv --python /usr/bin/python3
source .venv/bin/activate
uv pip install -r tests/regression/requirements.txt
python3 tests/regression/test_all.py --output-dir /tmp/actrail-regression
```

The requirements file installs Python workload dependencies used by regression cases, including LangGraph. Use a system-Python venv because the LangGraph TLS case needs an `_ssl` shared object that links dynamic OpenSSL. The runner framework itself is stdlib-only.

The runner prints colorized `PASS`/`FAIL`/`SKIP` rows and writes:

```text
/tmp/actrail-regression/report.md
/tmp/actrail-regression/report.json
```

Default mode is dependency-aware: missing optional agent tools or external API keys are `SKIP`; runnable case failures are `FAIL`. Use `--strict` when skipped cases should fail the run. Use `--suite agent`, `--suite payload`, `--suite enforcement`, or `--case <case-id>` to isolate a subset.

The runner auto-discovers common agent prerequisites before skipping: Node/OpenSSL Claude Code launchers, native ELF Claude entrypoints with OpenSSL symbols or configured Bun/static-BoringSSL discovery, opencode Bun binaries or launcher-adjacent binaries, xiaoO binaries on `PATH`, and Python interpreters with `langgraph`, `requests`, and dynamic OpenSSL. For opencode and supported native Claude binaries, if the checked-in Bun/BoringSSL map does not match the current binary, the runner attempts byte-pattern detection and generates a temporary map for the current build-id. For xiaoO HTTPS payload capture, rustls `PlaintextSink` symbols must be available from either the binary or debuginfo; a stripped xiaoO binary that reaches the provider through HTTP CONNECT proves only the proxy tunnel on the socket path and fails the payload case until debuginfo/TLS symbols or a plain HTTP provider route is supplied. Python builds with static or built-in `_ssl` are rejected for the LangGraph TLS case. If discovery cannot find a runnable dependency, read the check detail before treating the skip as a product failure. `CLAUDE_TLS_BINARY`, `OPENCODE_BIN_PATH`, `XIAOO_BINARY`, `XIAOO_DEBUG_FILE`, and `LANGGRAPH_PYTHON` remain explicit overrides; invalid override values fail fast.

## Example 01: Quick Start

Doc: `docs/examples/01.quick-start/README.md`

Purpose: process lifecycle and local TCP network events through `actraild`, `actrailctl`, `actrailviewer`, and `actrailweb`.

Run manually because the example intentionally teaches the attach workflow:

```bash
python3 docs/examples/clean.py --example quick-start
./target/release/actraild start --config docs/examples/01.quick-start/operator.conf
./target/release/actrailctl doctor --config docs/examples/01.quick-start/operator.conf
python3 docs/examples/01.quick-start/lifecycle_network_target.py
```

Copy the PID printed by the workload, then in another terminal:

```bash
./target/release/actrailctl track-add --config docs/examples/01.quick-start/operator.conf --pid <PID> --name quick-start-live
```

Return to the workload terminal and press Enter. Then verify:

```bash
./target/release/actrailviewer summary --config docs/examples/01.quick-start/operator.conf --trace-id <TRACE_ID>
./target/release/actrailviewer processes --config docs/examples/01.quick-start/operator.conf --trace-id <TRACE_ID>
./target/release/actrailviewer network --config docs/examples/01.quick-start/operator.conf --trace-id <TRACE_ID>
```

Stop daemon after the run:

```bash
./target/release/actraild stop --config docs/examples/01.quick-start/operator.conf
```

Expected result: trace enters `Active`, then `Completed`; viewer shows process lifecycle and local TCP `connect/accept/send/recv` events. A bootstrap-gap diagnostic is acceptable if documented in the README.

## Example 02: LLM HTTP Payload And Semantics

Doc: `docs/examples/02.llm-http-payload-capture/README.md`

Purpose: TLS plaintext payload capture and Application events.

No-network deterministic HTTP/2 path is manual and uses the files under `http2-local/`:

```bash
python3 docs/examples/clean.py --example http2-local
./target/release/actraild start --config docs/examples/02.llm-http-payload-capture/http2-local/operator.conf
./target/release/actrailctl launch \
  --config docs/examples/02.llm-http-payload-capture/http2-local/operator.conf \
  --name http2-local-transfer \
  -- \
  python3 docs/examples/02.llm-http-payload-capture/http2-local/workload.py \
  --target-config docs/examples/02.llm-http-payload-capture/http2-local/workload.conf
```

Record the trace id printed by `actrailctl launch`, then verify with:

```bash
./target/release/actrailviewer payloads --config docs/examples/02.llm-http-payload-capture/http2-local/operator.conf --trace-id <TRACE_ID> --head 40
./target/release/actrailviewer events --config docs/examples/02.llm-http-payload-capture/http2-local/operator.conf --trace-id <TRACE_ID> --tail 80
```

External OpenAI-compatible HTTP/1.1 path requires network and a provider API key. The defaults use DeepSeek and `DEEPSEEK_API_KEY`; set `ACTRAIL_LLM_BASE_URL`, `ACTRAIL_LLM_CHAT_PATH`, `ACTRAIL_LLM_MODEL`, `ACTRAIL_LLM_API_KEY_ENV`, or `ACTRAIL_LLM_REQUEST_JSON` to run against another compatible provider:

```bash
test -n "${DEEPSEEK_API_KEY:-}"
python3 docs/examples/clean.py --example llm-http1
./target/release/actraild start --config docs/examples/02.llm-http-payload-capture/external-openai-compatible/http1-operator.conf
./target/release/actrailctl launch \
  --config docs/examples/02.llm-http-payload-capture/external-openai-compatible/http1-operator.conf \
  --name llm-http1 \
  -- \
  bash docs/examples/02.llm-http-payload-capture/external-openai-compatible/http1.sh
```

External OpenAI-compatible HTTP/2 path has the same provider environment contract, plus an ALPN requirement: the active provider/CDN/proxy path must actually negotiate HTTP/2. If it negotiates HTTP/1.1, the regression marks this external HTTP/2 sub-check as skipped and relies on the external HTTP/1.1 path plus the deterministic local HTTP/2 workload.

```bash
test -n "${DEEPSEEK_API_KEY:-}"
python3 docs/examples/clean.py --example llm-http2
./target/release/actraild start --config docs/examples/02.llm-http-payload-capture/external-openai-compatible/http2-operator.conf
./target/release/actrailctl launch \
  --config docs/examples/02.llm-http-payload-capture/external-openai-compatible/http2-operator.conf \
  --name llm-http2 \
  -- \
  bash docs/examples/02.llm-http-payload-capture/external-openai-compatible/http2.sh
```

Expected result: payload rows are visible through `actrailviewer payloads`; HTTP/1.1 rows include an outbound `Application request` for `POST /chat/completions`; HTTP/2 rows include `Application frame` and `Application data` for a DATA frame carrying the request body when curl reports `ACTRAIL_CURL_HTTP_VERSION=2`. Do not require an inbound response row or HTTP/2 connection preface for this transfer test.

## Example 03: Extended Observation

Doc: `docs/examples/03.extended-observation-e2e/README.md`

Purpose: broad observation over process, file, mmap, IPC, network, stdio payload, provider labels, resource metrics, and Web UI data.

Use the manual workflow in the README as the transfer-test path. This case is passed by running a real workload and inspecting AcTrail's viewer output, not by relying only on the maintainer assertion harness.

```bash
python3 docs/examples/clean.py --example extended-observation
./target/release/actraild start --config docs/examples/03.extended-observation-e2e/operator.conf
./target/release/actrailctl doctor --config docs/examples/03.extended-observation-e2e/operator.conf
./target/release/ebpf_probe workload --config docs/examples/03.extended-observation-e2e/workload.conf
```

After the workload prints `workload_pid=<PID>`, run `actrailctl track-add` from another terminal, then return to the workload terminal and enter `actrail-stdio-stdin-e2e` and `actrail-stdio-continue-e2e` when prompted. Verify with `actrailviewer summary`, `processes`, `events --head 140`, `network --head 20`, and `payloads --head 12`.

Expected result: viewer output shows process fork/exec/exit and signal rows; File rows for regular file, path mutation, truncate, and `mmap_shared`; IPC rows for pipe, FIFO, and Unix socket; Net rows for bind/listen/connect/accept/send/recv; Label rows for `actrail-local-tcp`; Resource rows for `process_tree`; and Stdio payload rows whose `SOURCE` column is `Stdio`. `Completed/Degraded` with only the documented `BootstrapGap` diagnostic is acceptable for this manual attach workflow: it means the workload process existed before `track-add`, so AcTrail can snapshot the already-running process but cannot reconstruct pre-attach live eBPF history.

For maintainer regression only, `./target/release/ebpf_probe verify-live --config docs/examples/03.extended-observation-e2e/observation.conf` may be run as an additional assertion pass, but it is not the transfer-test substitute for inspecting viewer output.

## Example 04: Fanotify Enforcement

Doc: `docs/examples/04.fanotify-enforcement-e2e/README.md`

Purpose: trace-scoped file permission allow/deny decisions through fanotify.

Run:

```bash
python3 docs/examples/04.fanotify-enforcement-e2e/run_e2e.py \
  --otel-output /tmp/actrail-fanotify-e2e.otlp.json
```

Expected result: terminal shows `allowed=ok` and `denied=permission_denied` from the monitored agent process. Treat those two lines as the client-side proof: `allowed=ok` means the agent actually read the allowed file, and `denied=permission_denied` means the denied file raised `PermissionError`; `denied=unexpected_success` is a failure. Viewer output must also contain one allow and one deny `Enforcement` event, proving AcTrail recorded the fanotify decisions. With `--otel-output`, the script must also print `otel_enforcement_spans=allow,deny`; the exported file must contain two `enforcement.decision` spans: the allow span is success/OK, and the deny span is error. This example does not enable `stdio-chunk`, so the agent stdout is not expected to appear as trace payload. If the kernel lacks fanotify permission support, `actraild` fails fast at `fanotify_enforcement`.

## Example 05: HTTP Payload Unified

Doc: `docs/examples/05.http-payload-unified/README.md`

Purpose: non-TLS HTTP socket plaintext payload capture and the same payload/Application result surface used by HTTPS.

Run:

```bash
python3 docs/examples/clean.py --example http-payload
python3 docs/examples/05.http-payload-unified/run_e2e.py
```

Expected result: viewer output contains `Application request POST /plain-http`, `Application response 200 OK`, payload rows whose `SOURCE` column is `Syscall`, and payload text containing `POST /plain-http HTTP/1.1` plus `actrail-http-ok`. The non-HTTP marker `actrail-non-http` must not appear in retained payload text.

## Example 06: Claude Code LLM Payload Capture

Doc: `docs/examples/06.claude-code-tls-capture/README.md`

Purpose: complete LLM request capture and, for HTTPS/TLS runtimes, response capture from a real `claude -p ...` run via `actrailctl launch`. The traffic may be HTTPS/TLS or plain HTTP, depending on the Claude Code installation and proxy/provider configuration.

Preconditions:

- `claude` is on `PATH`.
- If Claude uses HTTPS, the installed `claude` entrypoint must be one of the TLS runtimes supported by the E2E resolver: Node/OpenSSL, native ELF OpenSSL symbols, or supported native/static-BoringSSL.
- If Claude uses a plain HTTP base URL/proxy, the socket plaintext backend is the expected capture path.
- The shell has whatever authentication Claude Code needs to answer `claude -p`.

Automated transfer test:

```bash
python3 docs/examples/clean.py --example claude-code
python3 tests/payload/claude-code/run_e2e.py \
  --config-template docs/examples/06.claude-code-tls-capture/operator.conf
```

Expected result: the script prints `claude code LLM payload e2e complete`, with nonzero `captured_payload_segments`, `exported_payload_nodes`, `semantic_action_rows`, and `otel_semantic_spans`. A run whose actual request rows are `TlsUserSpace` must also print nonzero `captured_response_payload_segments` from inbound `TlsUserSpace` rows. A valid request run may show payload rows from `TlsUserSpace` (`openssl`/`boringssl`) or from `Syscall` (`socket-syscall`) when Claude uses plain HTTP; plain HTTP does not require TLS response rows even if the installed Claude binary has a discoverable TLS runtime. The current Claude example uses `bpf-copy-seccomp-fallback` for TLS and socket payloads: BPF inline copy stays at the stable 4095-byte event ABI, while daemon-side process reads carry normal operations up to 4MB. Large request or response operations must show `captured == original` and `operation_completion_state=success`; LLM request/response rows marked `Truncated` fail the transfer test. The semantic action output must include a complete successful `llm.request`, and the OTLP JSON export must contain the corresponding span with split HTTP headers/body attributes: `llm.request.raw_payload_base64`, `http.request.headers_hpack_base64` or `http.request.headers_text`, `http.request.body_base64`, and `http.request.body_text`.

The configured Claude subprocess timeout is `claude_timeout_seconds = 180` in `tests/payload/claude-code/workload.conf`. The same file controls launch supervision with `launch_poll_interval_seconds` and `launch_stop_timeout_seconds`; if `actraild` exits while `actrailctl launch` is still running, the E2E fails immediately and prints daemon stdout/stderr. If the script times out, first run `timeout 120 claude -p "请用一句话回答：AcTrail 这个名字是否让你想到 actual trail？"` to separate Claude service latency from AcTrail capture behavior.

## Example 07: xiaoO Invokes Claude Code

Doc: `docs/examples/07.xiaoo-claude-agent-invocation/README.md`

Purpose: process-level semantic discovery when a real xiaoO agent launches `claude -p ...`.

Preconditions:

- `xiaoo` is on `PATH`; `which xiaoo` should print the executable that will be launched.
- `claude` is on `PATH` and authenticated.
- External network access is available for Claude Code.

Run:

```bash
python3 docs/examples/clean.py --example xiaoo-claude
python3 docs/examples/07.xiaoo-claude-agent-invocation/run_e2e.py
```

Expected result: terminal output contains `ACTRAIL_AGENT_TREE_OK`, `agent_invocation_trace_id=<TRACE_ID>`, and `agent invocation e2e complete`. The script exports pretty OTLP JSON to `/tmp/actrail-xiaoo-claude-agent-invocation.otlp.json` and validates that it contains a seccomp-observed `process.exec` span for `claude -p` plus an `agent.invocation` span whose parent executable is xiaoO and whose child command line contains `claude`.

This example intentionally disables TLS and socket payload capture. It validates the agent invocation tree and semantic action assembly, not Claude LLM request bytes.

## Real Agent Trace Suite

Doc: `tests/agent-trace/README.md`

Purpose: validate real LLM request capture and OTEL action export across multiple agent runtimes, not just one docs example.

Preconditions:

- `cargo build --release` completed.
- `claude` is on `PATH` and authenticated.
- `opencode` is on `PATH`.
- `xiaoo` is on `PATH`, or set `XIAOO_BINARY` when running the xiaoO case directly.
- `DEEPSEEK_API_KEY` is set for the LangGraph case.
- On proxy-only hosts, keep the shell's local proxy variables active. Do not unset `HTTP_PROXY`, `HTTPS_PROXY`, or `ALL_PROXY`.
- LangGraph uses a Python interpreter with `langgraph`, `requests`, and dynamic OpenSSL. If the default Python lacks those packages, create a system-Python venv:

```bash
python3 -m venv /tmp/actrail-langgraph-system-venv
/tmp/actrail-langgraph-system-venv/bin/python -m pip install langgraph requests
```

Run all cases:

```bash
LANGGRAPH_PYTHON=/tmp/actrail-langgraph-system-venv/bin/python \
  python3 tests/agent-trace/run_case.py all
```

Run individual cases when isolating a failure:

```bash
python3 tests/agent-trace/run_case.py claude-code
python3 tests/agent-trace/run_case.py opencode-bun
python3 tests/agent-trace/run_case.py xiaoo-rustls
LANGGRAPH_PYTHON=/tmp/actrail-langgraph-system-venv/bin/python \
  python3 tests/agent-trace/run_case.py langgraph-openai
```

Expected result:

- Claude Code prints `claude code LLM payload e2e complete`.
- opencode prints `opencode agent trace e2e complete`; the case is pinned to `deepseek/deepseek-chat` and should show a complete `llm.request` for `deepseek-chat`. HTTPS runs may also show `CONNECT api.deepseek.com:443`; plain HTTP runs should instead show `Syscall/socket-syscall` payload rows.
- xiaoO prints `xiaoO agent trace e2e complete` and shows a complete `llm.request` when either rustls plaintext symbols are available for HTTPS or xiaoO uses a plain HTTP provider route. A stripped xiaoO binary over HTTP CONNECT is a real unsupported payload-capture setup and should fail with a message asking for debuginfo/TLS symbols or a plain HTTP route.
- LangGraph prints `LangGraph OpenAI-compatible agent trace e2e complete` and shows `POST /chat/completions`.
- Each case prints nonzero payload and OTEL span counts. The required payload proof is a complete outbound plaintext payload row from either `TlsUserSpace` or `Syscall/socket-syscall`; the required semantic proof is a complete successful `llm.request` action plus an OTEL span whose `actrail.action.kind` is `llm.request`.

If the LangGraph workload uses HTTPS and no TLS payload rows appear, check the Python runtime first. A Python build that statically embeds OpenSSL leaves no dynamic `libssl` shared-library target for the current `openssl-symbols` resolver. If the workload uses a plain HTTP API URL, expect `Syscall/socket-syscall` payload rows instead.
