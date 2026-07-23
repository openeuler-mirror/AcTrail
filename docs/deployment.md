# AcTrail Deployment Guide

This document describes how to deploy AcTrail on a Linux or WSL host for live observation. It assumes release binaries are built from this repository.

## Deployment Shape

AcTrail has three runtime surfaces:

| Surface | Binary | Role |
| --- | --- | --- |
| Daemon | `actraild` | Loads collectors, receives control requests, writes storage. |
| Control CLI | `actrailctl` | Starts traces, launches children, lists/removes traces, cleans local artifacts. |
| Views and local administration | `actrailviewer`, `actrailweb` | Reads storage and renders observations; `actrailweb` can also scan and explicitly load/unload local plugin packages through the daemon control socket. |

Run one daemon per operator config. Keep each deployment's `socket_path`, `pid_file`, `storage_sqlite_path`, `log_path`, `export_directory`, plugin discovery directory, and enabled `otel-jsonl` route path unique.

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

The eBPF process fork path uses `sched/sched_process_fork`. The syscall tracepoint `syscalls/sys_enter_fork` is not a deployment prerequisite. Some architectures also omit compatibility fd-alias syscalls such as `syscalls/sys_enter_dup2`; AcTrail treats `dup2`/`dup3` fd-alias tracepoints as optional and continues with the core socket/file payload tracepoints when they are unavailable. Launch-time process seccomp resolves configured process-control names through the target architecture. For example, ARM64 targets that do not expose standalone `fork` or `vfork` syscalls use the available `clone`/`clone3` process-creation notifications instead of failing during filter construction.

## Operator Config Layout

Initialize the default full-collection operator config:

```bash
sudo ./target/release/actraild init
```

The default path is `/etc/actrail/actraild.conf`. `actrailctl init` performs the same initialization. If the file already exists, `init` loads and validates it, reports success or the validation error, and exits without rewriting it. Pass `--force` or `-f` to overwrite the target path. For a local deployment config, pass `--output local/operator.conf` or `--config local/operator.conf`. To initialize from the default template plus a TOML fragment, pass `init --patch <patch.toml>`.

The generated default keeps daemon runtime state and durable observation data out of `/tmp`:

| Field | Default |
| --- | --- |
| `socket_path` | `/run/actrail/control.sock` |
| `pid_file` | `/run/actrail/actraild.pid` |
| `payload_tls_sync_event_socket_path` | `/run/actrail/tls-sync.sock` |
| `storage_sqlite_path` | `/var/lib/actrail/actrail.sqlite` |
| `export_directory` | `/var/lib/actrail/export` |
| `plugins.discovery.directory` | `~/.actrail/plugins` |
| live `otel-jsonl` plugin config `path` | `/var/lib/actrail/export/live-spans.otlp.jsonl` when explicitly loaded |
| `log_path` | `/var/log/actrail/actraild.log` |
| `supervision.startup_wait_ms` | `30000` |
| `supervision.shutdown_wait_ms` | `5000` |
| `workload_diagnostics_enabled` | `false` |
| `workload_diagnostics_interval_ms` | `1000` |

`actraild` creates missing parent directories for configured daemon write paths before opening storage/log/export files or binding Unix sockets. Permission errors remain fatal; fix ownership/privileges or change the configured path deliberately.

For a persistent deployment, review these fields first:

| Field | Deployment Decision |
| --- | --- |
| `socket_path` | Local Unix socket used by `actrailctl`; keep it private to the host. |
| `socket_mode_octal` | File mode for the control socket. |
| `control_pending_connection_max` | Maximum number of simultaneously pending control socket clients; keep the default `256` unless one daemon must coordinate many concurrently connecting agents. |
| `active_trace_max` | Maximum number of simultaneously non-terminal traces admitted by one daemon. The default `128` admits 100 concurrent agent or local-command launches with headroom while bounding daemon collector state. |
| `pid_file` | Used by `actraild start/stop/status/restart`. |
| `storage_backend` | Storage backend implementation. Current supported value: `sqlite`. |
| `storage_sqlite_path` | SQLite storage location; place it on a filesystem with enough space for payload retention. |
| `storage_sqlite_busy_timeout_ms` | SQLite busy timeout for daemon writes. Keep this positive; increase it only when long-running readers share the same storage. |
| `log_path` | Daemon background stdout/stderr log. |
| `supervision.startup_wait_ms` | Maximum time that `start` or `restart` waits for both the PID file and control socket. The default is 30 seconds. |
| `supervision.shutdown_wait_ms` | Grace period for stopping a daemon, including cleanup after a startup failure, before forced termination. |
| `diagnostic_log_level` | Daemon diagnostic verbosity: `off`, `info`, or `debug`. Use `debug` only while collecting failure evidence. |
| `workload_diagnostics_enabled` | Enables periodic low-overhead daemon workload counter logs to help diagnose hot loops and projection/storage pressure. |
| `workload_diagnostics_interval_ms` | Period between workload diagnostic log lines when workload diagnostics are enabled. |
| `export_directory` | Default graph export directory when no explicit `--output` is passed. |
| `[plugins.discovery]` | Bounded package discovery for the Plugins Web workspace. The default directory is `~/.actrail/plugins`; it is resolved from the `actrailweb` process `HOME`, so use an absolute path when service identities differ. Discovery never loads a package. |
| `[plugins.startup]` | Startup plugin load list. The generated default is disabled and loads no plugins. Add an `otel-jsonl` startup load entry only when a realtime span stream is required. |
| `profile_name` and `required_capability` | Capability contract for traces created from this config. |
| `*_retention_max_bytes_per_trace` | Payload storage safety limits. |
| `export_payload_bytes_enabled` / `export_payload_text_enabled` | Whether raw payload bytes/text can appear in graph JSON export. |

AcTrail fails fast for unsupported required capabilities. Do not add broad fallback configs that silently reduce coverage.

`scripts/install-release.sh` builds and installs two official packages under `${ACTRAIL_PLUGIN_DIR:-$HOME/.actrail/plugins}`: `file-leakage/` provides post-trace leakage detection, and `file-policy-dynamic/` manages dynamic allow, deny, and gray file rules. Installation only makes these packages discoverable. The generated startup list remains empty and disabled; use the local Plugins Web workspace or the plugin CLI to load a package explicitly.

The dynamic file-policy plugin requests `file-policy.rules.apply`, whose grants must include allowed rule decisions and absolute path scopes. Select the candidate in the Web workspace, configure those grants in the load dialog, and load the instance. The daemon validates the submitted grants before activating the plugin. Web lifecycle operations are privileged daemon administration, so run `actrailweb` as an authorized local administrator and keep its listener on a trusted interface.

## Capability Profiles

The generated operator template is a broad collection profile similar to `docs/examples/08.full-monitor-validation/operator.conf`: process, file, IPC, stdio, TLS plaintext, socket plaintext, HTTP/1, HTTP/2, and resource metrics are enabled by default. Live OTEL export is not loaded by default; add an `otel-jsonl` plugin under `[plugins.startup]` or load it with `actraild plugin load` when a realtime span stream is required. Enforcement remains disabled because it is not passive collection and depends on deployment-specific rules.

Use a narrow config for each deployment intent:

| Intent | Required Capabilities / Settings |
| --- | --- |
| Process and network trail | `proc-lifecycle`, `net-transport`, `ebpf_enabled = true`. |
| File and IPC observation | Add `fs-access-basic`, `ipc-pipe-fifo`, `ipc-unix-socket`; set `file_path_capture_enabled = true` when path events are required. |
| Stdio payload | Add `stdio-chunk`; enable `payload_stdio_enabled` and the specific stdin/stdout/stderr booleans. |
| Plain HTTP payload | Add `socket-plaintext-payload`; enable `payload_socket_enabled`, `payload_socket_capture_backend`, `payload_socket_seccomp_syscall`, and HTTP application protocol settings. The default full-monitor config includes `write`, `writev`, `sendto`, and `sendmsg` so linear and vectored socket writes can both produce request payload evidence. |
| HTTPS OpenSSL payload | Add `tls-plaintext-payload`; configure TLS resolver/source and use `actrailctl launch` for `tls-sync` capture. |
| Agent LLM payload | Enable both HTTPS/TLS and plain HTTP socket payload paths when the provider route is not known in advance; the resulting `llm.request` evidence may come from `TlsUserSpace` or `Syscall/socket-syscall`. |
| Agent invocation discovery | Add `proc-exec-context` plus an LLM payload path; enable `seccomp_notify_enabled`, `process_seccomp_enabled`, payload capture, and `agent_invocation_enabled`. |
| Fanotify enforcement | Add `enforcement-file-permission-fanotify`; configure enforcement rules and run on a host with permission-event support. |

`observed-seccomp-agent`-style configs are not supersets of payload configs. Process seccomp observation and TLS payload capture solve different problems and should be enabled deliberately.

## Running The Daemon

Background mode:

```bash
./target/release/actraild --config local/operator.conf start
./target/release/actraild --config local/operator.conf status
./target/release/actrailctl doctor --config local/operator.conf
```

When `--config` is omitted, `actraild` and `actrailctl` load `/etc/actrail/actraild.conf`. If that file is missing or invalid, they fail with the config path and validation/read error.

`start` and `restart` report success only after both the configured PID file and control socket exist. Before binding the control socket, the daemon opens its configured services, runs the host eBPF load preflight for each capture profile, and loads startup plugins. These operations are part of `supervision.startup_wait_ms`; the generated default allows 30 seconds.

If readiness exceeds that limit or initialization fails, the operator terminates the spawned daemon, waits up to `supervision.shutdown_wait_ms`, force-stops it if necessary, and removes its PID and runtime socket files. The error identifies the readiness paths, whether the startup child exited or was stopped, and the configured `log_path`. A failed `start` therefore does not leave a child continuing initialization in the background.

Increase the startup limit only when successful preflight is consistently slower on the deployment host:

```toml
[supervision]
startup_wait_ms = 60000
shutdown_wait_ms = 5000
poll_interval_ms = 100
```

For an unexpected timeout, inspect `log_path` first. Repeated eBPF load failures, invalid plugin configuration, or storage errors require fixing the underlying startup failure rather than increasing the limit.

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
| Java JSSE payload | `payload_tls_java_agent_enabled = true` injects the embedded Java agent through `JAVA_TOOL_OPTIONS` during launch. Keep the default `false` outside Java JSSE workloads. |
| Process seccomp exec/fork/clone observation | The child process tree must inherit the configured seccomp user notification filter. Process-creation names are resolved through the target architecture's syscall map. |
| Agent invocation semantic actions | The daemon needs process exec context from the launch-time seccomp path. |

For existing processes, `track-add` can observe configured eBPF facts from the attach point onward, but it cannot install launch-time seccomp filters into the past.

## Storage And Retention

With `storage_backend = sqlite`, storage is append-oriented SQLite at `storage_sqlite_path`. The daemon opens file-backed storage in WAL mode and uses `storage_sqlite_busy_timeout_ms` when waiting on transient SQLite locks. Payload retention is controlled per trace by:

```text
payload_tls_retention_max_bytes_per_trace
payload_stdio_retention_max_bytes_per_trace
payload_socket_retention_max_bytes_per_trace
```

Stdio payload storage is controlled before persistence by `payload_stdio_stdin_storage_mode`, `payload_stdio_stdout_storage_mode`, and `payload_stdio_stderr_storage_mode`. Use `drop` for streams that should not be stored, and `metadata-only` when only segment timing/process metadata should be kept.

Socket BPF direct-copy is controlled by `payload_socket_max_segment_bytes`; the current stable socket BPF event ABI caps that inline copy at `4095` bytes. Larger socket operations fall back to user-read when `payload_socket_capture_backend = bpf-copy-seccomp-fallback`; that path is capped by `payload_socket_max_operation_bytes`. Generate the current default with `actrailctl init`, then override only when the deployment needs a different payload retention budget. Values above the configured operation cap are retained as partial/truncated payloads and must not be treated as complete `llm.request`. Vectored outbound socket syscalls (`writev` and `sendmsg`) are user-read fallback only because their payload is described by `iovec`/`msghdr` rather than a single linear buffer.

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
| `claude-code` | `tls-sync` auto-plan executable TLS payload when finder supports the local Claude runtime. | Complete outbound `TlsUserSpace <provider>` payload rows, `llm.request`, OTEL span. |
| `opencode-bun` | `tls-sync` auto-plan executable TLS payload, pinned to `deepseek/deepseek-chat`. | `CONNECT api.deepseek.com:443`, `POST /chat/completions`, complete outbound `TlsUserSpace <provider>` rows, `llm.request`, OTEL span. |
| `xiaoo-rustls` | `tls-sync` auto-plan rustls payload, or socket payload when xiaoO is configured for plain HTTP. | Complete outbound `TlsUserSpace rustls` rows for HTTPS, or complete outbound `Syscall/socket-syscall` rows for plain HTTP; then `llm.request` and OTEL span. A stripped HTTPS/HTTP CONNECT binary without a supported fast plan is expected to fail this payload case. |
| `langgraph-openai` | Python dynamic OpenSSL shared-library TLS payload. | `POST /chat/completions`, complete outbound `TlsUserSpace openssl` rows, `llm.request`, OTEL span. |

On proxy-only networks, do not unset the shell's local proxy variables. The opencode and LangGraph cases use real provider traffic and must inherit the same `HTTP_PROXY`, `HTTPS_PROXY`, or `ALL_PROXY` settings that make the agent work outside AcTrail.

For LangGraph, avoid Python builds that statically embed OpenSSL, because the `openssl-symbols` shared-library resolver has no dynamic `libssl` target to attach. A system-Python virtual environment with `langgraph` and `requests` is the expected deployment validation shape.

## Upgrade Procedure

1. Stop the daemon for the target config.
2. Build the new release binaries.
3. Run platform preflight on the host.
4. Review config template changes with `actraild init` or `actrailctl init` and update the deployment config explicitly.
5. Start the daemon and run a small example matching the deployed capability profile.

Do not rely on stale generated configs when new required configuration keys are introduced.
