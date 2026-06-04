# AcTrail Deployment Guide

This document describes how to deploy AcTrail on a Linux or WSL host for live observation. It assumes release binaries are built from this repository.

## Deployment Shape

AcTrail has three runtime surfaces:

| Surface | Binary | Role |
| --- | --- | --- |
| Daemon | `actraild` | Loads collectors, receives control requests, writes storage. |
| Control CLI | `actrailctl` | Starts traces, launches children, lists/removes traces, cleans local artifacts. |
| Read-only views | `actrailviewer`, `actrailweb` | Reads storage, renders events/payloads/actions, exports JSON or OTEL. |

Run one daemon per operator config. Keep each deployment's `socket_path`, `pid_file`, `storage_path`, `log_path`, `export_directory`, and enabled `otel_live_export_path` unique.

## Host Preconditions

Before deploying, run:

```bash
cargo build --release
python3 docs/preflight/platform_preflight.py --color always
```

For transfer-test readiness:

```bash
python3 docs/preflight/platform_preflight.py --run-smoke --color always
```

Required host capabilities are documented in [platform-requirements.md](platform-requirements.md). Distribution name is not enough; openEuler 24.03 and similar systems must still expose BTF, tracefs, permitted perf tracepoint attach, seccomp user notification, pidfd support, and any feature-specific kernel APIs such as fanotify permission events.

The eBPF process fork path uses `sched/sched_process_fork`. The syscall tracepoint `syscalls/sys_enter_fork` is not a deployment prerequisite. Some architectures also omit compatibility fd-alias syscalls such as `syscalls/sys_enter_dup2`; AcTrail treats `dup2`/`dup3` fd-alias tracepoints as optional and continues with the core socket/file payload tracepoints when they are unavailable.
Launch-time process seccomp resolves configured process-control names through the target architecture. For example, ARM64 targets that do not expose standalone `fork` or `vfork` syscalls use the available `clone`/`clone3` process-creation notifications instead of failing during filter construction.

## Operator Config Layout

Create a config template:

```bash
./target/release/actraild init-config --output local/operator.conf
```

For a persistent deployment, review these fields first:

| Field | Deployment Decision |
| --- | --- |
| `socket_path` | Local Unix socket used by `actrailctl`; keep it private to the host. |
| `socket_mode_octal` | File mode for the control socket. |
| `pid_file` | Used by `actraild start/stop/status/restart`. |
| `storage_path` | SQLite storage location; place it on a filesystem with enough space for payload retention. |
| `log_path` | Daemon background stdout/stderr log. |
| `diagnostic_log_level` | Daemon diagnostic verbosity: `off`, `info`, or `debug`. Use `debug` only while collecting failure evidence. |
| `export_directory` | Default graph export directory when no explicit `--output` is passed. |
| `otel_live_export_*` | Optional live OTEL JSONL sink. Keep disabled unless a realtime span stream is required. |
| `profile_name` and `required_capability` | Capability contract for traces created from this config. |
| `*_retention_max_bytes_per_trace` | Payload storage safety limits. |
| `export_payload_bytes_enabled` / `export_payload_text_enabled` | Whether raw payload bytes/text can appear in graph JSON export. |

AcTrail fails fast for unsupported required capabilities. Do not add broad fallback configs that silently reduce coverage.

## Capability Profiles

Use a narrow config for each deployment intent:

| Intent | Required Capabilities / Settings |
| --- | --- |
| Process and network trail | `proc-lifecycle`, `net-transport`, `ebpf_enabled = true`. |
| File and IPC observation | Add `fs-access-basic`, `ipc-pipe-fifo`, `ipc-unix-socket`; set `file_path_capture_enabled = true` when path events are required. |
| Stdio payload | Add `stdio-chunk`; enable `payload_stdio_enabled` and the specific stdin/stdout/stderr booleans. |
| Plain HTTP payload | Add `socket-plaintext-payload`; enable `payload_socket_enabled`, `payload_socket_capture_backend`, `payload_socket_seccomp_syscall`, and HTTP application protocol settings. |
| HTTPS OpenSSL payload | Add `tls-plaintext-payload`; configure TLS resolver/source and use `actrailctl launch` for `tls-sync` capture. |
| Agent LLM payload | Enable both HTTPS/TLS and plain HTTP socket payload paths when the provider route is not known in advance; the resulting `llm.request` evidence may come from `TlsUserSpace` or `Syscall/socket-syscall`. |
| Agent invocation discovery | Add `proc-exec-context`; enable `seccomp_notify_enabled`, `process_seccomp_enabled`, and `agent_invocation_enabled`. |
| Fanotify enforcement | Add `enforcement-file-permission-fanotify`; configure enforcement rules and run on a host with permission-event support. |

`observed-seccomp-agent`-style configs are not supersets of payload configs. Process seccomp observation and TLS payload capture solve different problems and should be enabled deliberately.

## Running The Daemon

Background mode:

```bash
./target/release/actraild --config local/operator.conf start
./target/release/actraild --config local/operator.conf status
./target/release/actrailctl doctor --config local/operator.conf
```

Foreground mode for a supervisor:

```bash
./target/release/actraild --config local/operator.conf run
```

Stop or restart:

```bash
./target/release/actraild --config local/operator.conf stop
./target/release/actraild --config local/operator.conf restart
```

Clean local runtime artifacts when intentionally resetting a test deployment:

```bash
./target/release/actrailctl clean --config local/operator.conf
```

Do not run `clean` against a config whose storage/log artifacts must be retained.

## Launch-Time Features

Some features require `actrailctl launch` because the child must be prepared before `exec`:

| Feature | Why `launch` Is Required |
| --- | --- |
| TLS sync payload (`tls-sync`) | `actrailctl launch` prepares the sync runtime, event socket, and probe plan before the child `exec`. Existing processes cannot receive that preload setup retroactively. |
| Process seccomp exec/fork/clone observation | The child process tree must inherit the configured seccomp user notification filter. Process-creation names are resolved through the target architecture's syscall map. |
| Agent invocation semantic actions | The daemon needs process exec context from the launch-time seccomp path. |

For existing processes, `track-add` can observe configured eBPF facts from the attach point onward, but it cannot install launch-time seccomp filters into the past.

## Storage And Retention

Storage is append-oriented SQLite at `storage_path`. Payload retention is controlled per trace by:

```text
payload_tls_retention_max_bytes_per_trace
payload_stdio_retention_max_bytes_per_trace
payload_socket_retention_max_bytes_per_trace
```

Socket BPF direct-copy is controlled by `payload_socket_max_segment_bytes`; the current stable socket BPF event ABI caps that inline copy at `4095` bytes. Larger socket operations fall back to user-read when `payload_socket_capture_backend = bpf-copy-seccomp-fallback`; that path is capped by `payload_socket_max_operation_bytes`. The default operation cap is `4194304` bytes, so HTTP LLM requests up to 4MB are still captured as complete plaintext without forcing every small socket event to reserve a fixed multi-MB ringbuf record. Values above the configured operation cap are retained as partial/truncated payloads and must not be treated as complete `llm.request`.

If export configs enable raw payload fields, graph JSON may contain sensitive request bodies. Keep `export_payload_bytes_enabled = false` and `export_payload_text_enabled = false` for routine deployments unless raw payload export is an explicit requirement.

## Operational Checks

After starting a deployment:

```bash
./target/release/actrailctl doctor --config local/operator.conf
./target/release/actrailctl list-traces --config local/operator.conf
./target/release/actrailviewer traces --config local/operator.conf
```

After a trace completes:

```bash
./target/release/actrailviewer summary --config local/operator.conf --trace-id <TRACE_ID>
./target/release/actrailviewer diagnostics --config local/operator.conf --trace-id <TRACE_ID>
```

Treat diagnostics as part of acceptance. `BootstrapGap` on a manual attach means pre-attach history may be incomplete; it is not by itself a payload-capture success or failure signal.

## Real Agent Acceptance

Use the real agent trace suite before handing a host to users who depend on LLM payload capture:

```bash
python3 tests/agent-trace/run_case.py claude-code
python3 tests/agent-trace/run_case.py opencode-bun
python3 tests/agent-trace/run_case.py xiaoo-rustls
LANGGRAPH_PYTHON=/path/to/dynamic-openssl-python \
  python3 tests/agent-trace/run_case.py langgraph-openai
```

These cases run compiled AcTrail binaries against real agent runtimes:

| Case | Runtime Path | Acceptance Evidence |
| --- | --- | --- |
| `claude-code` | Node/OpenSSL executable TLS payload. | Complete outbound `TlsUserSpace openssl` payload rows, `llm.request`, OTEL span. |
| `opencode-bun` | Bun/static-BoringSSL executable TLS payload, pinned to `deepseek/deepseek-chat`. | `CONNECT api.deepseek.com:443`, `POST /chat/completions`, complete outbound `TlsUserSpace boringssl` rows, `llm.request`, OTEL span. |
| `xiaoo-rustls` | Rust/rustls executable symbol-map TLS payload, or socket payload when xiaoO is configured for plain HTTP. | Complete outbound `TlsUserSpace rustls` rows for HTTPS, or complete outbound `Syscall/socket-syscall` rows for plain HTTP; then `llm.request` and OTEL span. A stripped HTTPS/HTTP CONNECT binary without debuginfo is expected to fail this payload case. |
| `langgraph-openai` | Python dynamic OpenSSL shared-library TLS payload. | `POST /chat/completions`, complete outbound `TlsUserSpace openssl` rows, `llm.request`, OTEL span. |

On proxy-only networks, do not unset the shell's local proxy variables. The opencode and LangGraph cases use real provider traffic and must inherit the same `HTTP_PROXY`, `HTTPS_PROXY`, or `ALL_PROXY` settings that make the agent work outside AcTrail.

For LangGraph, avoid Python builds that statically embed OpenSSL, because the `openssl-symbols` shared-library resolver has no dynamic `libssl` target to attach. A system-Python virtual environment with `langgraph` and `requests` is the expected deployment validation shape.

## Upgrade Procedure

1. Stop the daemon for the target config.
2. Build the new release binaries.
3. Run platform preflight on the host.
4. Review config template changes with `actraild init-config` and update the deployment config explicitly.
5. Start the daemon and run a small example matching the deployed capability profile.

Do not rely on stale generated configs when new required configuration keys are introduced.
