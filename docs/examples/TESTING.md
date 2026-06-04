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

The runner auto-discovers common agent prerequisites before skipping: Claude Code, opencode, xiaoO, and Python/LangGraph dependencies. A skipped row means the host is missing a required external tool, credential, or supported TLS/plain-HTTP payload path; read the skip detail before treating it as a product failure. `CLAUDE_TLS_BINARY`, `OPENCODE_BIN_PATH`, `XIAOO_BINARY`, `XIAOO_DEBUG_FILE`, and `LANGGRAPH_PYTHON` remain explicit overrides; invalid override values fail fast.

## Example 01: Quick Start

Doc: `docs/examples/01.quick-start/README.md`

Purpose: process lifecycle and local TCP network events through `actraild`, `actrailctl`, `actrailviewer`, and `actrailweb`.

Run manually because the example intentionally teaches the attach workflow:

```bash
python3 docs/examples/clean.py --example quick-start
./target/release/actraild --config docs/examples/01.quick-start/operator.conf start
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
./target/release/actraild --config docs/examples/01.quick-start/operator.conf stop
```

Expected result: trace enters `Active`, then `Completed`; viewer shows process lifecycle and local TCP `connect/accept/send/recv` events. A bootstrap-gap diagnostic is acceptable if documented in the README.

## Example 02: LLM HTTP Payload And Semantics

Doc: `docs/examples/02.llm-http-payload-capture/README.md`

Purpose: TLS plaintext payload capture and Application events.

No-network deterministic HTTP/2 path is manual and uses the files under `http2-local/`:

```bash
python3 docs/examples/clean.py --example http2-local
./target/release/actraild --config docs/examples/02.llm-http-payload-capture/http2-local/operator.conf start
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
./target/release/actraild --config docs/examples/02.llm-http-payload-capture/external-openai-compatible/http1-operator.conf start
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
./target/release/actraild --config docs/examples/02.llm-http-payload-capture/external-openai-compatible/http2-operator.conf start
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
./target/release/actraild --config docs/examples/03.extended-observation-e2e/operator.conf start
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

## Example 06: xiaoO TLS Payload Capture

Doc: `docs/examples/06.xiaoo-tls-capture/README.md`

Purpose: complete LLM request capture from a real `xiaoo run ...` launched through `actrailctl launch`. HTTPS/TLS traffic is captured through `tls-sync`; plain HTTP provider routes can be captured through socket plaintext.

Preconditions:

- `xiaoo` is on `PATH`; `which xiaoo` prints the executable under test.
- xiaoO has provider credentials configured and can answer a normal prompt.
- For HTTPS/TLS verification, finder fast returns a complete rustls plan:

```bash
target/release/tls-probe-point-finder fast --provider rustls --source auto xiaoo
```

Manual transfer test:

```bash
python3 docs/examples/clean.py --example xiaoo-tls
target/release/actraild --config docs/examples/06.xiaoo-tls-capture/operator.conf start
target/release/actrailctl --config docs/examples/06.xiaoo-tls-capture/operator.conf launch \
  --name xiaoo-tls-payload \
  -- \
  xiaoo run --no-tools --max-turns 1 --prompt "请直接回答：你好"
```

If the xiaoO CLI under test uses `-p` instead of `--prompt`, run the same command with `xiaoo run -p "请直接回答：你好"`.

Verify:

```bash
target/release/actrailviewer --config docs/examples/06.xiaoo-tls-capture/operator.conf payloads --trace-id <TRACE_ID> --head 80
target/release/actrailviewer --config docs/examples/06.xiaoo-tls-capture/operator.conf actions --trace-id <TRACE_ID> --head 80
```

Expected result: `payloads` shows a complete outbound plaintext payload row. HTTPS/TLS rows should use `SOURCE=TlsUserSpace` and `LIBRARY=rustls`; plain HTTP rows may use `SOURCE=Syscall` and `LIBRARY=socket-syscall`. A row that only contains HTTP proxy `CONNECT <host>:443` is not request body capture. `actions` must include a complete successful `llm.request`.

Stop daemon after the run:

```bash
target/release/actraild --config docs/examples/06.xiaoo-tls-capture/operator.conf stop
```

## Example 07: xiaoO Invokes Claude Code

Doc: `docs/examples/07.xiaoo-claude-agent-invocation/README.md`

Purpose: LLM-evidence-driven semantic discovery when a real xiaoO agent launches `claude -p ...`.

Preconditions:

- `xiaoo` is on `PATH`; `which xiaoo` should print the executable that will be launched.
- `claude` is on `PATH` and authenticated.
- External network access is available for Claude Code.

Run:

```bash
python3 docs/examples/clean.py --example xiaoo-claude
python3 docs/examples/07.xiaoo-claude-agent-invocation/run_e2e.py
```

Expected result: terminal output contains `ACTRAIL_AGENT_TREE_OK`, `agent_invocation_trace_id=<TRACE_ID>`, and `agent invocation e2e complete`. The script exports pretty OTLP JSON to `/tmp/actrail-xiaoo-claude-agent-invocation.otlp.json` and validates that it contains a seccomp-observed `process.exec` span for `claude -p`, an `llm.request` span for the same Claude process, and an `agent.invocation` span whose child command line contains `claude`. The invocation parent is Claude's direct launcher and may be a shell or timeout wrapper.

This example intentionally enables TLS payload capture because `agent.invocation` is generated from child LLM evidence, not from command names alone.

## Hidden Agent Invocation Regression

Doc: `tests/process/hidden-agent-invocation/README.md`

Purpose: verify LLM-evidence-driven agent identity when one agent-like process performs its own LLM request and then launches a hidden agent through an intermediate script.

Preconditions:

- `DEEPSEEK_API_KEY` is set.
- `xiaoo` is on `PATH`.
- `gcc` and OpenSSL headers/libraries are installed.
- External network access is available for DeepSeek and xiaoO.

Run:

```bash
python3 tests/process/hidden-agent-invocation/run_e2e.py --bin-dir target/release
```

Expected result: terminal output contains `HIDDEN_AGENT_A_LLM_OK`, `HIDDEN_AGENT_XIAOO_OK`, `hidden_agent_trace_id=<TRACE_ID>`, `agent_a_pid=<PID>`, `xiaoo_pid=<PID>`, `script_b_parent_pid=<PID>`, and `hidden agent invocation e2e complete`. The script validates three OTEL facts before passing: the root `agent_a` process is marked `agent.identity.status=observed`, the hidden `xiaoo` process is marked `agent.identity.status=observed`, and the `agent.invocation` edge uses the direct `script_b.sh -> xiaoo` parent/child relationship rather than an ancestor shortcut.

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
- xiaoO prints `xiaoO agent trace e2e complete` and shows a complete `llm.request` when either a rustls plaintext probe plan is available for HTTPS or xiaoO uses a plain HTTP provider route. For x86_64 stripped xiaoO builds, `tls-probe-point-finder fast --provider rustls --source auto xiaoo` should be the first check. If no rustls plan is available and the provider path is HTTP CONNECT, socket capture proves only the proxy tunnel and the case should ask for a supported rustls plan or a plain HTTP route.
- LangGraph prints `LangGraph OpenAI-compatible agent trace e2e complete` and shows `POST /chat/completions`.
- Each case prints nonzero payload and OTEL span counts. The required payload proof is a complete outbound plaintext payload row from either `TlsUserSpace` or `Syscall/socket-syscall`; the required semantic proof is a complete successful `llm.request` action plus an OTEL span whose `actrail.action.kind` is `llm.request`.

If the LangGraph workload uses HTTPS and no TLS payload rows appear, check the Python runtime first. A Python build that statically embeds OpenSSL leaves no dynamic `libssl` shared-library target for the current `openssl-symbols` resolver. If the workload uses a plain HTTP API URL, expect `Syscall/socket-syscall` payload rows instead.
