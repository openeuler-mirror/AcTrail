# Manual Test Walkthrough: Host actraild + Restricted Container actrailctl

This is a copy-paste-ready, step-by-step manual test for the degradation path described in [README.md](README.md). It assumes a host running Ubuntu-like glibc 2.39 with Docker, and an openEuler 24.03 image (glibc 2.38) for the workload container. Every step lists the command and the expected output so you can verify as you go.

Test goals: verify that with the workload container **not** given `--security-opt seccomp=unconfined` and **no CAP_BPF**, AcTrail still (1) starts the host daemon under `[ebpf] enabled = "auto"`, (2) reports `seccomp_launch=unavailable` from inside the container and recommends `skip`, (3) degrades `actrailctl launch` to tls-sync-only and captures TLS plaintext + `llm.*` semantic actions on the host.

> 中文版：[manual-test-walkthrough.zh.md](manual-test-walkthrough.zh.md)

## Where each command runs

Every code block below is tagged with where it must run:

- **`[host]`** — run in your normal host shell (the machine running Docker and `actraild`). These commands use host paths (`target/release/...`, `/etc/actrail`, `/var/lib/actrail`, `/run/actrail`) and host tools (`cargo`, `docker`, `sqlite3`, `curl`).
- **`[container]`** — run **inside** the workload container, via `docker exec actrail-restricted ...`. These commands see the container's filesystem (`/usr/local/bin`, `/root`, `/AcTrail`) and the mounted host sockets at `/run/actrail`.
- **`[host → container]`** — a host-side command of the form `docker exec ... bash -lc '...'` that launches a shell inside the container; the script body runs in the container.

Shell variables (`$TRACE_ID`, `$TRACE_NUM`, etc.) are set in earlier steps and reused later; set them in the same shell where you run the dependent command, or re-export them.

## Two machines, one container — what runs where

```text
┌──────────────────────────── HOST ────────────────────────────┐
│  actraild, actrailviewer, actrailweb  (release binaries)      │
│  /etc/actrail/actraild.conf   /var/lib/actrail/actrail.sqlite │
│  /run/actrail/{control.sock, tls-sync.sock}  ◄── mounted into │
│  Docker daemon                                                │
│                                                               │
│  You run: cargo build, actraild start/stop/status,           │
│           actrailctl doctor/list-traces, actrailviewer,       │
│           actrailweb, docker run/exec, sqlite3, curl          │
└───────────────────────────────────────────────────────────────┘
                          ▲ mount /run/actrail (RW), /etc/actrail (RO) ▲
┌──────────────────────── CONTAINER (actrail-restricted) ──────┐
│  openEuler 24.03, glibc 2.38, default seccomp, no CAP_BPF    │
│  actrailctl + libactrail_tls_payload_probe_sync.so (built    │
│    inside the container in Step 3)                            │
│  opencode agent + config (installed in Step 4)               │
│  /AcTrail — host source tree mounted RO (for building only)  │
│                                                               │
│  You run (via docker exec): actrailctl probe/launch,         │
│           dnf install, rustup, cargo build, opencode          │
└───────────────────────────────────────────────────────────────┘
```

## How to deploy and start the container (overview)

The container is created once (Step 2) and kept running with `tail -f /dev/null` so you can `docker exec` into it repeatedly across Steps 3-6. The essential options, and *why* each is needed:

| `docker run` option | Value | Why |
| --- | --- | --- |
| `--name` | `actrail-restricted` | Fixed name so every later `docker exec` can target it. |
| `--user` | `0:0` | Run as root inside the container; `actrailctl launch` needs to fork the child and set `LD_PRELOAD`. |
| `-v /run/actrail:/run/actrail` | RW bind mount | The container's `actrailctl` connects to the host daemon's control + tls-sync sockets through this mount. **Without it, no capture.** |
| `-v /etc/actrail:/etc/actrail:ro` | RO bind mount | The container reads the same operator config the host daemon uses, so socket paths and TLS settings match. |
| `-v "$(pwd)":/AcTrail:ro` | RO bind mount | Only needed for Step 3 (building `actrailctl` + `.so` inside the container). The source tree is mounted read-only; cargo writes to `/tmp` via `CARGO_TARGET_DIR`. Remove this mount after Step 3 if you want a leaner runtime container. |
| `openeuler/openeuler:24.03-lts-sp3` | image | glibc 2.38, matches the agent; distinct from the host glibc on purpose to demonstrate the in-container build. |
| `tail -f /dev/null` | entrypoint | Keeps the container alive so `docker exec` works across steps; the real agent is launched later via `actrailctl launch`. |

**Deliberately omitted options** (this is what makes the test a "restricted" test):

- **No `--security-opt seccomp=unconfined`** → Docker applies its default seccomp profile, which blocks `pidfd_getfd`. This is the constraint that forces the launch-time seccomp path to degrade to tls-sync-only.
- **No `--cap-add` (no `CAP_BPF`/`CAP_PERFMON`)** and **no `/sys/kernel` mount** → the container cannot do eBPF itself. (eBPF capture is done by the host daemon anyway, so this only matters if you expected the container to help.)

To **start** the container:

```bash
# [host]
docker run -d --name actrail-restricted \
  --user 0:0 \
  -v /run/actrail:/run/actrail \
  -v /etc/actrail:/etc/actrail:ro \
  -v "$(pwd)":/AcTrail:ro \
  openeuler/openeuler:24.03-lts-sp3 \
  tail -f /dev/null
```

To **enter** the container interactively (optional, for poking around between steps):

```bash
# [host → container]
docker exec -it actrail-restricted bash
```

Inside that shell you are in the container; `exit` returns to the host. The scripted steps below use non-interactive `docker exec ... bash -lc '...'` so each is one pasteable command.

To **stop/remove** the container (Step 9):

```bash
# [host]
docker rm -f actrail-restricted
```

> **Order matters:** create the host sockets first — Step 1 starts `actraild`, which creates `/run/actrail/*.sock` — **then** create the container so the bind mount sees the sockets. If you create the container before the daemon, the socket files will not exist inside the container even though the mount is present, and `actrailctl probe` will report `control_socket=unavailable`. Fix by starting `actraild` first.

---

## Step 0 — Preflight on the host

Confirm the host has what the test needs.

```bash
# [host]
# 0.1 AcTrail release binaries built fresh from current source
ls -la target/release/actraild target/release/actrailctl \
         target/release/actrailviewer target/release/actrailweb \
         target/release/libactrail_tls_payload_probe_sync.so

# 0.2 Kernel can run eBPF (this host will keep eBPF enabled under auto)
uname -a                       # expect aarch64/x86_64, kernel >= 5.10
test -f /sys/kernel/btf/vmlinux && echo "BTF ok"
test -d /sys/kernel/tracing && echo "tracefs ok"

# 0.3 Docker available
docker --version
docker ps --filter name=actrail-opencode-openeuler --format '{{.Names}} {{.Status}}'
# ^ this unconfined container is the source of actrailctl/.so/opencode binaries later.
#   If you do NOT have it, adapt Step 2 to provision the binaries another way.

# 0.4 You are root (eBPF + socket dirs need it)
id                             # expect uid=0(root)

# 0.5 Fresh build (skip only if 0.1 timestamps are newer than your last `git pull`)
cargo build --release -p daemon -p ctl -p view -p web -p tls_payload_probe_sync
```

Expected: all binaries exist; BTF + tracefs present; Docker responds; `actrail-opencode-openeuler` is Up; `id` shows root.

If the host cannot run eBPF (no BTF / non-root / no tracefs), the daemon auto-degrades at Step 1 and `doctor` will omit `ebpf` from collectors — the rest of the test still works because TLS plaintext comes from tls-sync, not eBPF.

---

## Step 1 — Install host config and start actraild

```bash
# [host]
# 1.1 Create runtime directories
install -d /etc/actrail /var/lib/actrail /run/actrail /var/log/actrail

# 1.2 Install the restricted example config (hierarchical TOML: ebpf enabled="auto",
#     process_seccomp enabled=false, tls-sync on)
install -m 0644 docs/examples/container-agent-restricted/operator.conf /etc/actrail/actraild.conf

# 1.3 Confirm the key fields (hierarchical TOML — keys live under [section] headers)
grep -nE '^\[control\]|^\[ebpf\]|^\[process_seccomp\]|^\[payload.tls\]|^\[storage.sqlite\]|^socket_path|^enabled|^binary_path|^capture_backend|^sync_event_socket_path|^path' /etc/actrail/actraild.conf
# Expect under [ebpf]:        enabled = "auto"
# Expect under [process_seccomp]: enabled = false
# Expect under [payload.tls]: enabled = true, capture_backend = "tls-sync", binary_path = "disabled",
#                            sync_event_socket_path = "/run/actrail/tls-sync.sock"
# Expect under [control]:     socket_path = "/run/actrail/control.sock"
# Expect under [storage.sqlite]: path = "/var/lib/actrail/actrail.sqlite"

# 1.4 Start the host daemon
./target/release/actraild --config /etc/actrail/actraild.conf start
```

Expected:
```
actraild started pid=<PID> socket=/run/actrail/control.sock
```

```bash
# [host]
# 1.5 Check status + sockets
./target/release/actraild --config /etc/actrail/actraild.conf status
ls -la /run/actrail/
```

Expected: `actraild running pid=...`; `/run/actrail/` contains `control.sock` and `tls-sync.sock` (both `srw-rw----`).

```bash
# [host]
# 1.6 Run doctor — confirm collectors + storage
./target/release/actrailctl --config /etc/actrail/actraild.conf doctor
```

Expected (host can run eBPF):
```
collectors=ebpf,tls-sync,application-protocol-analyzer plugins= storage_ready=true
```
If the host cannot run eBPF, the daemon log (`/var/log/actrail/actraild.log`) contains `actraild ebpf auto-degraded: <reason>; continuing without host eBPF collection` and `doctor` shows `collectors=tls-sync,application-protocol-analyzer` (no `ebpf`). The daemon did **not** refuse to start — that is the auto-degrade behavior.

<details>
<summary><b>Alternative: generate the host config from probe results instead of using the example file</b></summary>

If you would rather generate a config tailored to this host's actual probe results instead of installing the example `operator.conf`, run `probe --suggest-config` on the host. It works even before any config exists (first deploy):

```bash
# [host]
# 1.7 (alternative to 1.2) Generate a config trimmed to what the host probes found.
#     Prints to stdout; redirect to a file and install it. Review the header
#     summary first — it reflects seccomp/eBPF/tls-sync availability on this host.
./target/release/actrailctl probe --suggest-config > /tmp/suggested.conf
head -12 /tmp/suggested.conf          # inspect the probe summary + key fields
install -m 0644 /tmp/suggested.conf /etc/actrail/actraild.conf
```

When run **inside the restricted container** (Step 5), the same flag reflects the container's probes: `seccomp_launch=unavailable` causes the suggested config to set `[process_seccomp] enabled = false` and drop `proc-exec-context` from `[capture] capabilities`, so the host daemon starts without requiring seccomp. `--suggest-config` never writes a file itself — you redirect it.

</details>

---

## Step 2 — Create the restricted workload container

Create a container **without** `--security-opt seccomp=unconfined` (Docker default seccomp), **no** `/sys/kernel` override, mounting only `/run/actrail` (RW) and `/etc/actrail` (RO). Also mount the AcTrail source tree RO so the container can build `actrailctl` + the `.so` with its own glibc.

```bash
# [host]
# 2.1 Pull the image if you don't have it
docker image inspect openeuler/openeuler:24.03-lts-sp3 >/dev/null 2>&1 \
  || docker pull openeuler/openeuler:24.03-lts-sp3

# 2.2 Create the restricted container
docker run -d --name actrail-restricted \
  --user 0:0 \
  -v /run/actrail:/run/actrail \
  -v /etc/actrail:/etc/actrail:ro \
  -v "$(pwd)":/AcTrail:ro \
  openeuler/openeuler:24.03-lts-sp3 \
  tail -f /dev/null

docker ps --filter name=actrail-restricted --format '{{.Names}} {{.Status}}'
# -> actrail-restricted Up ...
```

Verify the container really is restricted:

```bash
# [host]
# 2.3 Docker default seccomp (NOT unconfined) — inspect is a host-side Docker query
docker inspect actrail-restricted --format '{{json .HostConfig.SecurityOpt}}'
# -> null          (means: default profile, NOT seccomp=unconfined)

# [host → container]
#     grep runs inside the container; docker exec is the host-side launcher.
docker exec actrail-restricted grep -i seccomp /proc/self/status
# -> Seccomp: 2     (filter mode, active)
# -> Seccomp_filters: 1

# [host → container]
# 2.4 Sockets mounted from host — these test commands run inside the container
docker exec actrail-restricted test -S /run/actrail/control.sock && echo "control.sock ok"
docker exec actrail-restricted test -S /run/actrail/tls-sync.sock && echo "tls-sync.sock ok"
docker exec actrail-restricted test -r /etc/actrail/actraild.conf && echo "conf readable"
```

Expected: `SecurityOpt` is `null`; `Seccomp: 2`; both sockets `ok`; conf `readable`.

---

## Step 3 — Build actrailctl + the TLS-sync .so inside the container

The host-compiled `actrailctl` depends on the host glibc (e.g. 2.39) and fails in the openEuler container (glibc 2.38) with `GLIBC_2.39 not found`. Build inside the container instead. This needs only Rust + gcc/openssl/zlib devel (no clang/llvm — eBPF C compilation lives in the daemon, already built on the host).

```bash
# [host → container]
# 3.1 Install build deps (one-time, ~1-2 min)
docker exec actrail-restricted bash -lc '
  dnf install -y --setopt=install_weak_deps=False \
    gcc make pkgconf-pkg-config openssl-devel zlib-devel perl
  gcc --version | head -1
'
```

Expected: ends with `gcc (GCC) 12.3.1 (...)` (version may differ).

```bash
# [host → container]
# 3.2 Install Rust via rustup with a fast mirror (one-time, ~1-2 min)
docker exec actrail-restricted bash -lc '
  export RUSTUP_DIST_SERVER=https://mirrors.tuna.tsinghua.edu.cn/rustup
  export RUSTUP_UPDATE_ROOT=https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup
  curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup.sh
  sh /tmp/rustup.sh -y --default-toolchain stable --profile minimal
  rm -f /tmp/rustup.sh
  /root/.cargo/bin/rustc --version
'
```

Expected: prints a rustc version (e.g. `rustc 1.96.0 ...`).

```bash
# [host → container]
# 3.3 Configure cargo to use a mirror, then build ctl + tls_payload_probe_sync
#     CARGO_TARGET_DIR is under /tmp because /AcTrail is mounted read-only.
docker exec actrail-restricted bash -lc '
  set -e
  export RUSTUP_HOME=/root/.rustup CARGO_HOME=/root/.cargo
  export PATH=/root/.cargo/bin:$PATH
  export CARGO_TARGET_DIR=/tmp/actrail-target
  mkdir -p /root/.cargo
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
```

Expected: `Finished \`release\` profile [optimized] target(s) in ...`. This takes ~5-10 min the first time (compiling all deps).

```bash
# [host → container]
# 3.4 Verify the built ctl runs and has probe + seccomp-mode
docker exec actrail-restricted /usr/local/bin/actrailctl --help | grep -iE "probe|launch"
docker exec actrail-restricted /usr/local/bin/actrailctl launch --help | grep -i seccomp
# -> --seccomp-mode <SECCOMP_MODE>  [default: auto] [possible values: auto, require, skip]

# 3.5 Confirm glibc compatibility (no missing libs)
docker exec actrail-restricted ldd /usr/local/bin/actrailctl | grep -i "not found" || echo "no missing libs"
docker exec actrail-restricted ldd /usr/local/bin/libactrail_tls_payload_probe_sync.so | grep -i "not found" || echo "so: no missing libs"
```

Expected: `probe` and `launch` subcommands present; `--seccomp-mode` default `auto`; `ldd` reports no missing libraries.

---

## Step 4 — Install the agent (opencode) into the container

If your restricted container image does not already contain the agent, install it. Here we copy `opencode` + its config from the existing unconfined container `actrail-opencode-openeuler` via a host relay.

```bash
# [host]
# 4.1 Export opencode + its config from the unconfined container to a host temp dir.
#     `mkdir`, `docker cp`, `tar` run on the host; the `docker exec ... bash -lc`
#     body runs in the unconfined *source* container.
mkdir -p /tmp/actrail-agent-pkg
docker exec actrail-opencode-openeuler bash -lc '
  cd /usr/local/bin && tar -cf /tmp/opencode.tar opencode
  cd /root/.config/opencode && tar -rf /tmp/opencode.tar opencode.json
'
docker cp actrail-opencode-openeuler:/tmp/opencode.tar /tmp/actrail-agent-pkg/opencode.tar
tar -C /tmp/actrail-agent-pkg -xf /tmp/actrail-agent-pkg/opencode.tar

# 4.2 Install into the restricted container (host-side docker cp; chmod/mkdir
#     execute inside the restricted container via docker exec)
docker cp /tmp/actrail-agent-pkg/opencode actrail-restricted:/usr/local/bin/opencode
docker exec actrail-restricted chmod 0755 /usr/local/bin/opencode
docker exec actrail-restricted mkdir -p /root/.config/opencode
docker cp /tmp/actrail-agent-pkg/opencode.json actrail-restricted:/root/.config/opencode/opencode.json

# 4.3 Verify (opencode --version runs inside the restricted container)
docker exec actrail-restricted /usr/local/bin/opencode --version
# -> 1.15.13  (or your version)
docker exec actrail-restricted test -x /usr/local/bin/opencode && echo "opencode ok"
```

> If your agent is not opencode, replace the binary path and config in 4.1-4.2 and the `opencode run "..."` invocations in later steps with your agent's non-interactive command.

---

## Step 5 — Run actrailctl probe (verify seccomp unavailable + recommend skip)

```bash
# [host → container]
# 5.1 Human-readable probe — actrailctl runs inside the container
docker exec actrail-restricted \
  actrailctl --config /etc/actrail/actraild.conf probe
```

Expected:
```
unix_socket=ok connected /run/actrail/control.sock
unix_socket=ok connected /run/actrail/tls-sync.sock
no_new_privs=ok enabled=0
seccomp_launch=unavailable pidfd_getfd seccomp listener: Operation not permitted (os error 1)
tls_sync_runtime_library=ok found /usr/local/bin/libactrail_tls_payload_probe_sync.so
collectors=ebpf,tls-sync,application-protocol-analyzer plugins= storage_ready=true
launch_seccomp_mode=skip
launch_note=seccomp launch path unavailable; use --seccomp-mode auto (default) for tls-sync-only launch
```

The key lines: `seccomp_launch=unavailable` (default seccomp blocks `pidfd_getfd`), `launch_seccomp_mode=skip`, and `launch_note` recommending `--seccomp-mode auto`.

```bash
# [host → container]  (probe --json runs in the container; output is redirected to a HOST file)
# 5.2 JSON (for assertions)
docker exec actrail-restricted \
  actrailctl --config /etc/actrail/actraild.conf probe --json > /tmp/probe.json

# [host]
cat /tmp/probe.json | python3 -m json.tool | head -30

# 5.3 Assertions (python3 runs on the host, reading the host-side /tmp/probe.json)
python3 - <<'PY'
import json
d = json.load(open('/tmp/probe.json'))
socks = [x for x in d['statuses'] if x['name'] == 'unix_socket']
assert len(socks) == 2 and all(x['available'] for x in socks), "sockets not ok"
sec = {x['name']: x for x in d['statuses']}['seccomp_launch']
assert not sec['available'], "seccomp_launch should be unavailable"
assert 'pidfd_getfd' in sec['detail'] and 'Operation not permitted' in sec['detail']
assert {x['name']: x for x in d['statuses']}['tls_sync_runtime_library']['available']
assert d['recommended_seccomp_mode'] == 'skip'
assert 'tls-sync-only' in d['launch_note']
print("probe assertions ALL PASSED")
PY
```

Expected: `probe assertions ALL PASSED`.

```bash
# [host → container]
# 5.4 Local-only probe (skip daemon doctor)
docker exec actrail-restricted \
  actrailctl --config /etc/actrail/actraild.conf probe --skip-daemon
```

Expected: same local checks, but no `collectors=...` line (daemon doctor skipped).

---

## Step 6 — Launch with --seccomp-mode auto (degrades to tls-sync-only)

```bash
# [host → container]
# 6.1 Default auto mode — should degrade and succeed.
#     actrailctl launch runs inside the container; the agent (opencode) is its child.
docker exec actrail-restricted bash -lc '
  export PATH=/usr/local/bin:$PATH
  export LD_LIBRARY_PATH=/usr/local/bin:$LD_LIBRARY_PATH
  actrailctl --config /etc/actrail/actraild.conf launch -- \
    /usr/local/bin/opencode run "用一句话回答：AcTrail 是什么"
'
```

Expected (first lines on stderr):
```
actrailctl launch degraded: pidfd_getfd seccomp listener: Operation not permitted (os error 1); continuing with tls-sync-only launch without socket/process seccomp
trace trace-<N> entered Active
```
Then the agent runs (calls the model, prints a response) and exits 0.

> Note the `<N>` — this is the trace id you will use in Step 7. If you started from an empty DB it is `trace-1`; on this host there are already older traces, so it may be `trace-4` or higher. Use `list-traces` (Step 7.1) to find the newest one.

Controls (same container) to contrast the three modes:

```bash
# [host → container]
# 6.2 require — must FAIL outright (no silent degradation)
docker exec actrail-restricted bash -lc '
  export PATH=/usr/local/bin:$PATH
  actrailctl --config /etc/actrail/actraild.conf launch --seccomp-mode require -- \
    /usr/local/bin/opencode run "hi"
'
# [host]
echo "require exit=$?"
# -> non-zero exit, last line: "pidfd_getfd seccomp listener: Operation not permitted (os error 1)"
```

```bash
# [host → container]
# 6.3 skip — must succeed (equivalent to auto in this restricted env)
docker exec actrail-restricted bash -lc '
  export PATH=/usr/local/bin:$PATH
  actrailctl --config /etc/actrail/actraild.conf launch --seccomp-mode skip -- \
    /usr/local/bin/opencode run "说一个字: 好"
'
# [host]
echo "skip exit=$?"
# -> exit 0, stderr: "actrailctl launch degraded: seccomp launch disabled by --seccomp-mode skip"
```

---

## Step 7 — Verify host-side capture (tls-sync still delivers plaintext + semantics)

```bash
# [host]
# 7.1 List traces; pick the newest Exited one (from 6.1, the auto run)
./target/release/actrailctl --config /etc/actrail/actraild.conf list-traces
```

Expected: a list ending with the trace you just launched, e.g. `trace-4 pid-<PID> pid=<PID> Exited/Clean`.

```bash
# [host]
# 7.2 Set TRACE_ID to the trace from 6.1 (the auto-degraded, full-question run).
#     These variables are used by 7.3, 7.4 and Step 8 — keep them in the same shell.
TRACE_ID=trace-4        # <-- replace with your newest Exited trace id
TRACE_NUM=${TRACE_ID#trace-}
echo "TRACE_ID=$TRACE_ID TRACE_NUM=$TRACE_NUM"
```

```bash
# [host]
# 7.3 Summary — actrailviewer reads the host sqlite
./target/release/actrailviewer --config /etc/actrail/actraild.conf summary --trace-id "$TRACE_ID"
```

Expected: `state=Exited health=Clean`, non-zero `processes`, `events`, `network_events`.

```bash
# [host]
# 7.4 TLS plaintext payload_segments + semantic_actions (counts only, no sensitive body).
#     sqlite3 opens the host /var/lib/actrail/actrail.sqlite directly.
sqlite3 /var/lib/actrail/actrail.sqlite "
select library, symbol, direction, count(*) as cnt, sum(captured_size) as bytes
  from payload_segments where trace_id = $TRACE_NUM
  group by library, symbol, direction order by library, symbol, direction;
select '---semantic---';
select kind, count(*) as cnt from semantic_actions where trace_id = $TRACE_NUM
  group by kind order by kind;
"
```

Expected: `payload_segments` has `boringssl SSL_read` (inbound) and `boringssl SSL_write` (outbound) rows for opencode (BoringSSL). `semantic_actions` has `llm.request`, `llm.response`, `llm.call`, plus `command.invocation`, `process.exec`, `http.message`, `sse.stream`. This is the core result: **no seccomp/eBPF privileges in the container, yet TLS plaintext + LLM semantics were captured on the host via tls-sync.**

---

## Step 8 — Verify the web UI action graph

```bash
# [host]
# 8.1 Start the web UI bound to all interfaces on port 9999 (background)
./target/release/actrailweb --config /etc/actrail/actraild.conf --addr 0.0.0.0 --port 9999 &
echo "web pid=$!"
sleep 2
```

Expected: `actrailweb listening on http://0.0.0.0:9999 storage=/var/lib/actrail/actrail.sqlite`.

```bash
# [host]
# 8.2 Open in a browser
#     Find the host IP first:
ip -4 addr show | grep -oP 'inet \K[0-9.]+' | grep -v '^127\.'
#     Then open:  http://<one-of-those-IPs>:9999/
#     Select the trace from Step 7.2 to see the action tree, payloads, commands.
```

```bash
# [host]
# 8.3 API checks from the host (bypass the shell proxy with --noproxy)
curl --noproxy '*' -fsS "http://127.0.0.1:9999/api/traces/$TRACE_NUM/action-tree" \
  | jq -r '[(.roots|length),(.actions|length),(.links|length)]|@tsv'
curl --noproxy '*' -fsS "http://127.0.0.1:9999/api/traces/$TRACE_NUM/commands" | jq -r 'length'
```

Expected: three numbers for the action tree (roots ≥ 1, actions ≥ 1, links ≥ 0) and a non-zero commands count.

> If `curl` returns `502` or `000`, your shell sets `http_proxy`/`https_proxy`. Use `--noproxy '*'` (as above) or `NO_PROXY=localhost,127.0.0.1 curl ...`. Browsers usually do not use these shell proxies, so the UI opens fine.

```bash
# [host]
# 8.4 Stop the web UI when done (find and kill it)
pkill -f 'actrailweb.*9999'
```

---

## Step 9 — Cleanup

```bash
# [host]
# 9.1 Stop the host daemon
./target/release/actraild --config /etc/actrail/actraild.conf stop

# 9.2 Remove the restricted container (host-side Docker command)
docker rm -f actrail-restricted

# 9.3 Remove the agent package temp dir
rm -rf /tmp/actrail-agent-pkg /tmp/probe.json

# 9.4 (Optional) wipe the SQLite store to start fresh next time
# rm -f /var/lib/actrail/actrail.sqlite
```

The `/etc/actrail/actraild.conf` and runtime dirs are retained for the next run.

---

## Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| `invalid ebpf.enabled: expected true, false, or auto, got ...` | `target/release/actraild` is older than the `[ebpf] enabled = "auto"` support. | Rebuild: `cargo build --release -p daemon -p ctl -p view -p web -p tls_payload_probe_sync` (Step 0.5). |
| `missing config key payload_stdio_capture_stdin` | Using a stale example config (e.g. old `container-agent-minimal/operator.conf`). | Use `docs/examples/container-agent-restricted/operator.conf` (it matches the current daemon). |
| `GLIBC_2.39 not found` when running `actrailctl` in the container | Host-compiled binary depends on a newer glibc than the container has. | Build `actrailctl` + `.so` inside the container (Step 3). |
| `tls-sync auto plan requires payload.tls.binary_path=disabled` | `[payload.tls] binary_path` is set to a path under tls-sync backend. | It must be `disabled` under tls-sync. The restricted example config already sets this; do not change it. |
| `seccomp_launch=ok` in the container | The container was started with `--security-opt seccomp=unconfined`. | Recreate without that flag (Step 2.2). This test specifically requires the default profile. |
| No TLS payload rows in `payload_segments` | Agent not started via `actrailctl launch`; `.so` missing; socket not mounted; or the agent's TLS library is not BoringSSL/rustls. | Re-check Steps 2.4, 3.4, 6.1; confirm the agent uses a supported TLS library. |
| `curl` to the web UI returns `502` | Shell `http_proxy`/`https_proxy` intercepting localhost. | Use `curl --noproxy '*'` or `NO_PROXY=localhost,127.0.0.1`. |
