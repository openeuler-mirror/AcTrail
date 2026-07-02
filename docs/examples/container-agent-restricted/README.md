# Container Agent, Restricted (No eBPF / No seccomp Privileges)

This example tests the **degradation path** that lets AcTrail keep observing an agent when the workload container is locked down: Docker's **default seccomp profile** (not `seccomp=unconfined`) and **no `/sys/kernel` mount / `CAP_BPF`**. It is the counterpart to [container-agent-minimal](../container-agent-minimal/README.md): minimal describes the config baseline; this one proves the degradation chain end to end.

## What it verifies

| Function point | How it is verified here |
| --- | --- |
| eBPF on/off switch + auto detection | Host `actraild` runs with `[ebpf] enabled = "auto"`. It probes eBPF at startup: on a host that can run eBPF it stays enabled; on a host that cannot it prints `actraild ebpf auto-degraded: ...` and continues instead of refusing to start. |
| `actrailctl probe` self-check + recommended launch | Inside the restricted container, `probe` reports `seccomp_notify=unavailable` (default seccomp blocks `pidfd_getfd`), `launch_seccomp_notify=disabled`, and `launch_note` recommending `--seccomp-notify auto` for a non-notify deployment — while `control_socket` and `tls_sync_socket` stay `ok`. `probe --suggest-config` prints a minimal operator config (to stdout) trimmed to the probe results: seccomp unavailable ⇒ `[process_seccomp] enabled = false` and `proc-exec-context` dropped; works even with no config file yet. |
| `actrailctl launch` seccomp degradation | Default `--seccomp-notify auto` degrades to tls-sync-only launch (stderr reports that seccomp-notify was disabled by the resolved deployment policy), the trace still enters Active, and the agent runs. `--seccomp-notify required` fails outright; `--seccomp-notify disabled` succeeds. |
| Capture without eBPF/seccomp privileges | Even with the container locked down, the host side still captures TLS plaintext (`boringssl SSL_read/SSL_write` or `rustls_*`) and `llm.*` semantic actions via the tls-sync backend, plus an action graph in the web UI. |

## Topology

```text
host actraild  ([ebpf] enabled="auto", [process_seccomp] enabled=false, tls-sync enabled)
  -> /run/actrail/control.sock  +  /run/actrail/tls-sync.sock   (mounted RW into container)
  -> /var/lib/actrail/actrail.sqlite
  -> actrailviewer / actrailweb

Docker workload container  (default seccomp, no --security-opt seccomp=unconfined, no /sys/kernel)
  -> actrailctl probe  (reports seccomp_notify=unavailable, recommends auto)
  -> actrailctl launch --seccomp-notify auto -- <agent>   (degrades to tls-sync-only)
  -> child agent in container PID namespace
```

The container is intentionally created **without** `--security-opt seccomp=unconfined` so Docker applies its default seccomp profile. That profile blocks the `pidfd_getfd` syscall the launch-time seccomp user-notify path needs, which is exactly the constraint this example exercises.

## How the degraded path captures TLS plaintext without seccomp

The reason seccomp was originally required is that the socket/process capture path **hijacks system calls** (seccomp user-notify intercepts `write/writev/sendto/sendmsg`, and process seccomp intercepts `exec*`). When the container's default seccomp profile blocks that path, AcTrail degrades to the **tls-sync** path, which does **not** hijack syscalls at all — it hooks the TLS library functions from *inside* the agent process address space. The two paths are independent, and the downgrade only drops the syscall-interception channel, not the plaintext channel.

```text
┌─────────────────────────────────── host ───────────────────────────────────┐
│                                                                             │
│  actraild                                                                    │
│   │                                                                          │
│   │ 1a. resolve_ebpf_collector_config([ebpf] enabled="auto")                     │
│   │     probe() → host can run eBPF? keep enabled                            │
│   │                host cannot?     print "actraild ebpf auto-degraded: ..." │
│   │                                  set ebpf_config.enabled=false, CONTINUE  │
│   │                                                                          │
│   │ 1b. bind /run/actrail/control.sock    (control plane)                     │
│   │     bind /run/actrail/tls-sync.sock   (plaintext event ingest)  ◄──┐     │
│   │                                                                    │ 8   │
│   │ 7. TlsSyncService.drain():                                         │     │
│   │      accept_ready_clients()  ← new child connects via inherited fd│     │
│   │      read_events()  →  decode_event_line()  →  RawPayloadSegment   │     │
│   │      ingest → payload_segments table (boringssl SSL_read/SSL_write)│     │
│   │      HTTP/semantic analyzers → llm.request / llm.response / ...    │     │
│   │                                                                    │     │
│   └─► /var/lib/actrail/actrail.sqlite  →  actrailviewer / actrailweb   │     │
│                                                                        │     │
└────────────────────────────────────────────────────────────────────────┼─────┘
                                          ▲                             │
                                          │ 6. plain bytes written to  │
                                          │    inherited event fd       │
                                          │    (libc::write, no stdio)  │
┌──────────────────────────────────── container (default seccomp, no CAP_BPF)─┼─────┐
│                                          │                             │     │
│  actrailctl launch --seccomp-notify auto -- <agent>                        │     │
│   │                                                                      │     │
│   │ 2. run_platform_probe_from_launch(): probe_seccomp_notify_capability │     │
│   │      → pidfd_getfd → EPERM (default seccomp blocks it)               │     │
│   │    daemon resolves permission policy (auto, unavailable):            │     │
│   │      selected.seccomp_notify=false, degraded=true                    │     │
│   │      deployment_permission_reasons=seccomp_notify_unavailable: ...   │     │
│   │                                                                      │     │
│   │ 3. ChildSetup::Plain (no seccomp filter installed)                   │     │
│   │    sync_launch(): probe-point finder resolves SSL_read/SSL_write     │     │
│   │      (or rustls_buffer_plaintext) addresses in the agent binary      │     │
│   │                                                                      │     │
│   │ 4. InheritableSuppressedFd::connect_unix_socket(tls-sync.sock)       │     │
│   │      → ctl connects to host ◄────────────────────────────────────────┼──┐  │
│   │      → clear FD_CLOEXEC so the fd survives exec                      │  │  │
│   │                                                                      │  │  │
│   │ 5. fork+exec agent with:                                             │  │  │
│   │      LD_PRELOAD=libactrail_tls_payload_probe_sync.so ──────────────┐ │  │  │
│   │      ACTRAIL_* env (trace_id, event_fd, limits)                    │ │  │  │
│   │                                                                    │ │  │  │
│   │   ┌───────────────────── agent process ────────────────────────┐  │ │  │  │
│   │   │                                                              │  │ │  │  │
│   │   │  .so (preloaded) installs native inline hooks:              │  │ │  │  │
│   │   │   allocate_trampoline (mmap RWX)                            │  │ │  │  │
│   │   │   write_trampoline (relocate stolen instructions)          │  │ │  │  │
│   │   │   patch_target (mprotect RWX, write LDR x16;BR x16)        │  │ │  │  │
│   │   │       at SSL_write / SSL_read / rustls_* entry             │  │ │  │  │
│   │   │                                                              │ └─┼─┘  │
│   │   │  agent calls SSL_write(plaintext_buf, n)                   │    │    │
│   │   │    → jumps into .so replacement fn                         │    │    │
│   │   │    → captures plaintext_buf BEFORE TLS encryption          │    │    │
│   │   │    → encode_event_line():                                    │    │    │
│   │   │        v1\tpayload\ttrace_id\tpid\tdir\tprovider\t         │    │    │
│   │   │        symbol\tstream_key\tseq\t<hex bytes>\n              │    │    │
│   │   │    → write(event_fd, line)  ─────────────────────────────┐ │    │    │
│   │   │    → call original SSL_write (trampoline) → real TLS     │ │    │    │
│   │   │                                                            │ │    │    │
│   │   └──────────────────────────────────────────────────────────┼┘    │    │
│   │                                                                │     │    │
│   │  (agent runs normally; payload capture is invisible to it)    │     │    │
│   │                                                                ▼     │    │
│   │                                                          step 6 ─────┘    │
│   │                                                                 ▲          │
│   │                                                                 └── step 5  │
│   │                                                                              │
│   │  (compare: --seccomp-notify required → permission resolution returns Err      │
│   │   with the pidfd_getfd EPERM; --seccomp-notify disabled → same Plain path,    │
│   │   selected explicitly by deployment policy)                                  │
│   │                                                                              │
└──────────────────────────────────────────────────────────────────────────────────┘
```

### Why this works without seccomp/eBPF privileges in the container

| Container lacks | Affects tls-sync? | Why |
| --- | --- | --- |
| `--security-opt seccomp=unconfined` (default profile blocks `pidfd_getfd`) | **No** | tls-sync never installs a seccomp filter. The hook is `LD_PRELOAD` + `mprotect`, both ordinary process operations. The default profile only blocks the *seccomp user-notify* path, which the downgrade abandons. |
| `CAP_BPF` / tracefs not mounted | **No** | eBPF is a separate collector (process/network/socket) run by the **host daemon**, not in the container. Its absence only drops those channels; the plaintext channel is unaffected. |
| `/run/actrail` not mounted | **Yes** | The `.so` cannot deliver events to the host `tls-sync.sock`; no plaintext reaches storage. |
| `.so` missing / glibc-incompatible | **Yes** | No hook can be injected into the agent process. |

The capture point moves from **kernel-mode syscall interception** (seccomp user-notify) to **user-mode function inline hooking** (LD_PRELOAD + native inline hook). That is a different, weaker privilege model: it needs only what every unprivileged container already has — an inherited unix-socket fd, `LD_PRELOAD`, and `mmap`/`mprotect` with `PROT_EXEC`.

### Stage-by-stage source reference

| Stage | Code | What it does |
| --- | --- | --- |
| 1a eBPF auto probe | `crates/apps/daemon/src/ebpf_resolve.rs` (`auto_follows_probe_result`) | `[ebpf] enabled = "auto"` → `probe().reason_unavailable` ⇒ `config.enabled=false`, `auto_degraded=true`, returns Ok (daemon does not abort). |
| 1b bind sockets | `crates/apps/daemon/src/services/tls_sync/service.rs` (`TlsSyncService::new`) | Binds `/run/actrail/tls-sync.sock` (non-blocking `UnixListener`). |
| 2 probe + degrade | `crates/apps/ctl/src/launch.rs` + `launch/permission_policy.rs` | The platform probe feeds permission resolution; `auto` plus unavailable seccomp-notify selects that axis off and records a machine-readable degradation reason. |
| 3 plain spawn + plan | `crates/apps/ctl/src/launch.rs` (`ChildSetup::Plain`, `seccomp_setup` skipped) + `launch/sync.rs` (`sync_launch`) | `tls_probe_point_finder::fast::resolve` finds `SSL_read/SSL_write`/`rustls_*` offsets; no seccomp filter installed. |
| 4 inherited event fd | `crates/apps/ctl/src/launch/suppress.rs` (`InheritableSuppressedFd`) | ctl connects to host `tls-sync.sock`, clears `FD_CLOEXEC` so the fd survives `exec`. |
| 5 inject + hook | `crates/apps/ctl/src/launch/sync.rs` (`sync_launch_envs`, sets `LD_PRELOAD`) + `crates/tools/tls_payload_probe_sync/src/runtime/hook/{aarch64,x86_64}.rs` | `.so` preloaded into agent; installs inline hook (`mmap` trampoline, `mprotect` target page, write `LDR x16,[literal]; BR x16` at function entry). |
| 6 emit event | `.so` runtime + `crates/core/tls_payload_sync/src/event.rs` (`encode_event_line`) | Captures plaintext buffer, encodes `v1\tpayload\t...`, `libc::write` to inherited fd; then calls original function via trampoline. |
| 7 ingest + project | `crates/apps/daemon/src/services/tls_sync/service.rs` (`drain`/`read_events`/`drain_complete_lines`) + payload/HTTP/semantic analyzers | `decode_event_line` → `RawPayloadSegment` → `payload_segments` rows; analyzers project `llm.request`/`llm.response`/`llm.call`, `http.message`, `sse.stream`, etc. |
| 8 view | `actrailviewer` / `actrailweb` read `/var/lib/actrail/actrail.sqlite` | Summary, action tree, payloads, commands. |

### The three seccomp modes, side by side

| `--seccomp-notify` | seccomp available | seccomp unavailable (this example) |
| --- | --- | --- |
| `auto` (default) | installs seccomp filter, full capture | **degrades**: selects seccomp-notify off, reports `seccomp_notify_unavailable`, trace enters Active, plaintext still captured via tls-sync |
| `required` | installs seccomp filter, full capture | **fails outright** with the `pidfd_getfd ... Operation not permitted` detail |
| `disabled` | skips seccomp (tls-sync-only by choice) | skips seccomp (tls-sync-only) by explicit deployment policy |

## Prerequisites

- Host has release binaries built fresh from the current source: `actraild`, `actrailctl`, `actrailviewer`, `actrailweb`, `libactrail_tls_payload_probe_sync.so`. **Rebuild after every pull** — a stale `target/release/actraild` may predate `[ebpf] enabled = "auto"` support and reject the config with `invalid ebpf.enabled: expected true, false, or auto`.
- Host can run eBPF (has `/sys/kernel/btf/vmlinux` and `/sys/kernel/tracing`). On such a host the auto probe keeps eBPF enabled; to simulate a host without eBPF, see "Simulate host without eBPF" below.
- A workload image that already contains the agent binary (here `opencode`) and its config. `actrailctl` and the TLS-sync `.so` are built **inside the container** because the host-compiled binaries depend on the host glibc (e.g. Ubuntu 24.04 glibc 2.39) which is newer than the container's (openEuler 24.03 glibc 2.38); a host-built `actrailctl` fails with `GLIBC_2.39 not found`.

## Steps

> For a copy-paste-ready, command-by-command manual with expected outputs and a troubleshooting table, see [manual-test-walkthrough.md](manual-test-walkthrough.md) (English) or [manual-test-walkthrough.zh.md](manual-test-walkthrough.zh.md) (中文). The steps below are the same flow in summary form.

### 1. Install the host config and start actraild

```bash
sudo install -d /etc/actrail /var/lib/actrail /run/actrail /var/log/actrail
sudo install -m 0644 docs/examples/container-agent-restricted/operator.conf /etc/actrail/actraild.conf
sudo ./target/release/actraild --config /etc/actrail/actraild.conf start
sudo ./target/release/actrailctl --config /etc/actrail/actraild.conf doctor
```

Expected: daemon starts; `/run/actrail/control.sock` and `/run/actrail/tls-sync.sock` exist; `doctor` reports `collectors=ebpf,tls-sync,application-protocol-analyzer storage_ready=true` on a host that can run eBPF. If the host cannot run eBPF, the daemon log contains `actraild ebpf auto-degraded: <reason>; continuing without host eBPF collection` and `doctor` omits `ebpf` from collectors — but the daemon still starts.

### 2. Create a restricted workload container

Create the container **without** `--security-opt seccomp=unconfined`, mount only `/run/actrail` (RW) and `/etc/actrail` (RO). Do **not** mount `/sys/kernel`. The example below assumes an image that already contains `opencode` and its config at `/root/.config/opencode/opencode.json`; if yours does not, copy them in first.

```bash
docker run -d --name actrail-restricted \
  --user 0:0 \
  -v /run/actrail:/run/actrail \
  -v /etc/actrail:/etc/actrail:ro \
  -v "$(pwd)":/AcTrail:ro \
  <your-agent-image> \
  tail -f /dev/null
```

Verify the container really is restricted:

```bash
# Docker default seccomp (NOT unconfined)
docker inspect actrail-restricted --format '{{json .HostConfig.SecurityOpt}}'   # -> null
docker exec actrail-restricted grep -i seccomp /proc/self/status               # -> Seccomp: 2

# Sockets mounted
docker exec actrail-restricted test -S /run/actrail/control.sock && echo ok
docker exec actrail-restricted test -S /run/actrail/tls-sync.sock && echo ok
```

### 3. Build actrailctl + the TLS-sync .so inside the container

This is required when the host glibc is newer than the container glibc. The build needs only Rust + gcc/openssl/zlib devel (no clang/llvm — eBPF C compilation lives in the daemon, already built on the host).

```bash
docker exec actrail-restricted bash -lc '
  dnf install -y --setopt=install_weak_deps=False gcc make pkgconf-pkg-config openssl-devel zlib-devel perl
  # rustup via a fast mirror (adjust if your network prefers another)
  export RUSTUP_DIST_SERVER=https://mirrors.tuna.tsinghua.edu.cn/rustup
  export RUSTUP_UPDATE_ROOT=https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup
  curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup.sh
  sh /tmp/rustup.sh -y --default-toolchain stable --profile minimal
  rm -f /tmp/rustup.sh
'
```

Point cargo at a mirror to speed up dependency downloads, then build only `ctl` and `tls_payload_probe_sync` (the daemon and web are not needed in the container). Use a writable `CARGO_TARGET_DIR` because the source tree is mounted read-only:

```bash
docker exec actrail-restricted bash -lc '
  set -e
  export RUSTUP_HOME=/root/.rustup CARGO_HOME=/root/.cargo
  export PATH=/root/.cargo/bin:$PATH
  export CARGO_TARGET_DIR=/tmp/actrail-target
  cat > /root/.cargo/config.toml <<EOF
[source.crates-io]
replace-with = "tuna"
[source.tuna]
registry = "sparse+https://mirrors.tuna.tsinghua.edu.cn/crates.io-index/"
[net]
git-fetch-with-cli = true
EOF
  cd /AcTrail
  cargo build --release -p ctl -p tls_payload_probe_sync
  install -m 0755 /tmp/actrail-target/release/actrailctl /usr/local/bin/actrailctl
  install -m 0755 /tmp/actrail-target/release/libactrail_tls_payload_probe_sync.so /usr/local/bin/libactrail_tls_payload_probe_sync.so
  rm -rf /tmp/actrail-target
'
docker exec actrail-restricted /usr/local/bin/actrailctl --help | grep -iE "probe|launch"
docker exec actrail-restricted ldd /usr/local/bin/actrailctl | grep -i "not found" || echo "no missing libs"
```

Expected: `actrailctl` runs in the container, the `probe` and `launch` subcommands are present, and `ldd` reports no missing libraries.

### 4. Run actrailctl probe

```bash
docker exec actrail-restricted actrailctl --config /etc/actrail/actraild.conf probe
docker exec actrail-restricted actrailctl --config /etc/actrail/actraild.conf probe --json
docker exec actrail-restricted actrailctl --config /etc/actrail/actraild.conf probe --skip-daemon
```

Expected (human output):

```
unix_socket=ok connected /run/actrail/control.sock
unix_socket=ok connected /run/actrail/tls-sync.sock
no_new_privs=ok enabled=0
seccomp_notify=unavailable pidfd_getfd seccomp listener: Operation not permitted (os error 1)
tls_sync_runtime_library=ok found /usr/local/bin/libactrail_tls_payload_probe_sync.so
collectors=ebpf,tls-sync,application-protocol-analyzer plugins= storage_ready=true
launch_seccomp_notify=disabled
launch_note=seccomp-notify unavailable; use --seccomp-notify auto (default) to select a non-notify deployment
```

JSON assertions (run on the host):

```bash
docker exec actrail-restricted actrailctl --config /etc/actrail/actraild.conf probe --json > /tmp/probe.json
python3 - <<'PY'
import json
d = json.load(open('/tmp/probe.json'))
socks = [x for x in d['statuses'] if x['name'] == 'unix_socket']
assert len(socks) == 2 and all(x['available'] for x in socks)
sec = {x['name']: x for x in d['statuses']}['seccomp_notify']
assert not sec['available'] and 'pidfd_getfd' in sec['detail'] and 'Operation not permitted' in sec['detail']
assert d['launch_seccomp_notify'] is False
assert 'non-notify deployment' in d['launch_note']
print('probe OK')
PY
```

### 5. Launch the agent with --seccomp-notify auto (degrades to tls-sync-only)

```bash
docker exec actrail-restricted bash -lc '
  export PATH=/usr/local/bin:$PATH
  export LD_LIBRARY_PATH=/usr/local/bin:$LD_LIBRARY_PATH
  actrailctl --config /etc/actrail/actraild.conf launch -- \
    /usr/local/bin/opencode run "用一句话回答：AcTrail 是什么"
'
```

Expected: permission output includes `deployment_permissions_degraded=true` and `deployment_permission_reasons=seccomp_notify_unavailable: ...pidfd_getfd...`, followed by `trace trace-<N> entered Active`; the agent completes and exits 0.

Controls (same container):

```bash
# required: must FAIL (no silent degradation)
docker exec actrail-restricted actrailctl --config /etc/actrail/actraild.conf launch --seccomp-notify required -- /usr/local/bin/opencode run "hi"
# -> non-zero exit, "pidfd_getfd seccomp listener: Operation not permitted"

# disabled: must succeed
docker exec actrail-restricted actrailctl --config /etc/actrail/actraild.conf launch --seccomp-notify disabled -- /usr/local/bin/opencode run "说一个字: 好"
# -> exit 0, deployment_permissions_degraded=false
```

### 6. Verify host-side capture (tls-sync still delivers plaintext + semantics)

```bash
sudo ./target/release/actrailctl --config /etc/actrail/actraild.conf list-traces
TRACE_ID=trace-2     # the auto-degraded run from step 5
TRACE_NUM=${TRACE_ID#trace-}
sudo ./target/release/actrailviewer --config /etc/actrail/actraild.conf summary --trace-id "$TRACE_ID"
sudo sqlite3 /var/lib/actrail/actrail.sqlite "
select library, symbol, direction, count(*), sum(captured_size)
  from payload_segments where trace_id = $TRACE_NUM
  group by library, symbol, direction order by library, symbol, direction;
select kind, count(*) from semantic_actions where trace_id = $TRACE_NUM
  group by kind order by kind;
"
```

Expected: `payload_segments` has `boringssl SSL_read/SSL_write` (opencode uses BoringSSL) or `rustls_*` rows; `semantic_actions` has `llm.request`, `llm.response`, `llm.call`, plus `command.invocation`, `process.exec`, `http.message`, `sse.stream`. The trace summary shows non-zero process/event/network counts and `state=Exited health=Clean`.

### 7. Verify the web UI action graph

```bash
sudo ./target/release/actrailweb --config /etc/actrail/actraild.conf --addr 127.0.0.1 --port 18080
```

Open `http://127.0.0.1:18080/` in a browser, select the trace, and inspect the action tree, payload evidence, and diagnostics. If your shell sets `http_proxy`/`https_proxy`, bypass it for localhost (`curl --noproxy '*' ...` or `NO_PROXY=localhost,127.0.0.1`) — otherwise curl returns 502 from the proxy instead of the web UI.

API checks:

```bash
curl --noproxy '*' -fsS "http://127.0.0.1:18080/api/traces/$TRACE_NUM/action-tree" | jq -r '[(.roots|length),(.actions|length),(.links|length)]|@tsv'
curl --noproxy '*' -fsS "http://127.0.0.1:18080/api/traces/$TRACE_NUM/commands" | jq -r 'length'
```

Expected: roots ≥ 1, actions ≥ 1, links ≥ 0, commands ≥ 1 — proving a real action graph was captured, not just argv/stdout.

### 8. Stop services

```bash
sudo ./target/release/actraild --config /etc/actrail/actraild.conf stop
docker rm -f actrail-restricted
```

The SQLite store at `/var/lib/actrail/actrail.sqlite` is retained for later inspection.

## Notes

- **`[payload.tls] binary_path` must stay `disabled`** under `[payload.tls] capture_backend = "tls-sync"`. The sync backend builds the probe plan dynamically at launch; a fixed binary path is rejected with `tls-sync auto plan requires payload.tls.binary_path=disabled`. The `binary_path` option is only a fallback for the non-sync executable-source path.
- The agent here is `opencode` (Go + BoringSSL). AcTrail captures `boringssl SSL_read/SSL_write` plaintext for it. For a `rustls`-based agent (e.g. xiaoO) expect `rustls_buffer_plaintext` / `rustls_take_received_plaintext` instead. Either way `llm.*` semantic actions are projected on the host daemon side from the captured LLM HTTP/TLS application payload.
- The container can still "see" `/sys/kernel/tracing` and `/sys/kernel/btf/vmlinux` through Docker's default `/sys` bind mount — that does **not** mean it can do eBPF: the container lacks `CAP_BPF`/`CAP_PERFMON`, and the real capture is on the host daemon regardless. The constraint this example exercises is the **default seccomp profile** blocking the launch-time `pidfd_getfd` path.

## Simulate host without eBPF

On a host that *can* run eBPF (so `[ebpf] enabled = "auto"` stays enabled and you do not see the degrade message), you can still confirm the auto-degrade code path does not refuse startup. The cleanest proof is a host or VM without BTF / without `CAP_BPF`. As a smoke check on a capable host, run the daemon in the foreground with a copy of the config and watch the startup line:

```bash
sudo ./target/release/actraild --config /etc/actrail/actraild.conf run
# on a host that can run eBPF: no "auto-degraded" line
# on a host that cannot: "actraild ebpf auto-degraded: <reason>; continuing without host eBPF collection"
```

The degrade decision is covered by `crates/apps/daemon/src/ebpf_resolve.rs` (`auto_follows_probe_result`): when the probe reports `reason_unavailable`, the resolution sets `config.enabled = false` and `auto_degraded = true` and returns `Ok` — the daemon does not abort.
