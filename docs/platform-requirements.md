# AcTrail Platform Requirements And Preflight

This document is for transfer-test and deployment validation. Run commands from the repository root unless a step says otherwise.

## Support Scope

AcTrail's eBPF collector build currently supports these Linux architectures:

| Architecture | Status | Notes |
| --- | --- | --- |
| `x86_64` | Verified in current development transfer tests | Current docs examples were run on x86_64 WSL. |
| `aarch64` / ARM64 | Intended supported target | The eBPF build, uprobe register reader, and process-seccomp syscall mapping contain ARM64 paths. Run the full transfer matrix on the target OS before claiming support. |
| 32-bit ARM or other architectures | Not supported | The eBPF build fails fast for architectures other than `x86_64` and `aarch64`. |

Linux distributions such as openEuler 24.03 are acceptable targets only when the kernel and runtime expose the capabilities below. Do not treat the distribution name as sufficient proof.

## One-Command Preflight

Run this first on every transfer-test host:

```bash
python3 docs/preflight/platform_preflight.py --color always
```

Expected:

- green `✓` means the checked item is usable.
- red `✗` on a `[required]` item blocks transfer testing.
- yellow `!` means the script can see the prerequisite but has not run the live runtime proof.

For the full local runtime proof, run:

```bash
python3 docs/preflight/platform_preflight.py --run-smoke --color always
```

The smoke mode runs the documented local eBPF live attach, HTTP/2 TLS sync payload, and fanotify enforcement checks. It also scans `claude` and `opencode` if they are on `PATH`, resolves their runtime executable when possible, and checks whether the executable exports the required OpenSSL symbols `SSL_read`, `SSL_write`, `SSL_read_ex`, and `SSL_write_ex`. OpenSSL `SSL_write_ex2` is reported as an optional outbound probe point when present. Agent executable rows are marked `[optional]` because they only block the corresponding agent-specific example.

## Required Capabilities

| Capability | Used By | How To Validate |
| --- | --- | --- |
| Root or equivalent capabilities | All live collection and fanotify examples | `id -u` must print `0`, or the service must run with equivalent kernel capabilities. Containers may hide capabilities even when the user is root. |
| Kernel BTF | eBPF CO-RE programs | `test -r /sys/kernel/btf/vmlinux` |
| Writable tracefs control mount | eBPF tracepoint attachment | `grep -w tracefs /proc/self/mountinfo` and `test -w /sys/kernel/tracing || test -w /sys/kernel/debug/tracing` |
| Tracepoint/uprobe attachment | Process, file, network, stdio, socket payload, and legacy uprobe TLS examples | `./target/release/ebpf_probe verify-live --config docs/examples/03.extended-observation-e2e/observation.conf` |
| TLS sync preload runtime | Current TLS plaintext payload examples with `payload_tls_capture_backend = tls-sync` | Run the local HTTP/2 TLS payload example through `actrailctl launch`. |
| Seccomp user notification | Process exec context and socket payload fallback paths | Run the process invocation or socket payload examples that enable those paths. |
| `pidfd_open` and `pidfd_getfd` | Copying a child seccomp listener before exec when seccomp-backed paths are enabled | Covered by process invocation and socket fallback examples. |
| `process_vm_readv` access to traced child | Reading large socket fallback operations and legacy seccomp-backed payload operations | Covered by the examples that enable those paths; failures appear in daemon logs or missing payload rows. |
| Fanotify permission events | Example 04 enforcement | `python3 docs/examples/04.fanotify-enforcement-e2e/run_e2e.py` |

The development WSL machine used `sysctl kernel.perf_event_paranoid=-1` for live examples, then restored `kernel.perf_event_paranoid=2` after testing. Use the target environment's security policy; AcTrail should fail fast instead of silently downgrading when the kernel refuses required features.

## Build Preflight

Install native build dependencies before running Cargo:

```bash
# openEuler/Fedora/RHEL-like
sudo dnf install -y clang llvm elfutils-devel zlib-devel pkgconf-pkg-config openssl-devel

# Debian/Ubuntu-like
sudo apt-get install -y clang llvm libelf-dev zlib1g-dev pkg-config libssl-dev
```

```bash
uname -m
id -u
cargo build --release
```

Expected:

- `uname -m` is `x86_64` or `aarch64`.
- The build completes and produces `target/release/actraild`, `actrailctl`, `actrailviewer`, and `ebpf_probe`.
- If the build fails while compiling the eBPF object, install the target distribution's equivalents for Clang/LLVM, libelf development headers, zlib development headers, pkg-config, and OpenSSL development headers, then rebuild.

## Kernel Preflight

```bash
test -r /sys/kernel/btf/vmlinux
grep -w tracefs /proc/self/mountinfo
test -w /sys/kernel/tracing || test -w /sys/kernel/debug/tracing
sysctl kernel.perf_event_paranoid kernel.unprivileged_bpf_disabled
```

Expected:

- `/sys/kernel/btf/vmlinux` exists and is readable.
- tracefs is mounted at `/sys/kernel/tracing` or `/sys/kernel/debug/tracing`.
- the tracefs control mount is writable by the process that runs `actraild`.
- sysctl values permit the required tracepoint/uprobe attachment for the test user.

## eBPF Live Preflight

This is the broadest local smoke test for process, file, mmap, IPC, network, resource, provider-label, and stdio payload collection:

```bash
python3 docs/examples/clean.py --example extended-observation
./target/release/ebpf_probe verify-live \
  --config docs/examples/03.extended-observation-e2e/observation.conf
```

Expected result includes:

```text
live verification passed
process_events=exec,exit,fork
file_events=...
net_events=...
ipc_events=...
resource_events=process_tree
provider_events=actrail-local-tcp
stdio_payloads=stderr:outbound,stdin:inbound,stdout:outbound
```

If this fails before attach, do not ask testers to edit configs by hand. Treat it as a platform prerequisite failure or an implementation failure. AcTrail's process fork observation uses `sched/sched_process_fork`; the syscall tracepoint `syscalls/sys_enter_fork` is not required for this preflight. Default process lifecycle capture suppresses process signal events such as `SIGCHLD`. Some target kernels do not expose compatibility fd-alias tracepoints such as `syscalls/sys_enter_dup2`. AcTrail treats `dup2`/`dup3` alias tracepoints as optional: their absence can reduce fd alias fidelity, but it must not block process, network, file, or socket-payload collection. For launch-time process seccomp, config values such as `fork` and `vfork` are resolved through the target architecture's syscall map. Architectures without standalone `fork` or `vfork` syscalls use the available process-creation syscalls such as `clone` and `clone3`; emitted trace metadata still records the actual syscall that fired.

## TLS Sync Preflight

This validates the launch-time TLS sync payload path without external network or API keys. The local HTTP/2 example uses `payload_tls_capture_backend = tls-sync`.

```bash
python3 docs/examples/clean.py --example http2-local
./target/release/actraild \
  --config docs/examples/02.llm-http-payload-capture/http2-local/operator.conf \
  start
./target/release/actrailctl launch \
  --config docs/examples/02.llm-http-payload-capture/http2-local/operator.conf \
  --name http2-local-transfer \
  -- \
  python3 docs/examples/02.llm-http-payload-capture/http2-local/workload.py \
  --target-config docs/examples/02.llm-http-payload-capture/http2-local/workload.conf
./target/release/actrailviewer payloads \
  --config docs/examples/02.llm-http-payload-capture/http2-local/operator.conf \
  --trace-id 1 \
  --head 40
./target/release/actrailviewer events \
  --config docs/examples/02.llm-http-payload-capture/http2-local/operator.conf \
  --trace-id 1 \
  --tail 80
./target/release/actraild \
  --config docs/examples/02.llm-http-payload-capture/http2-local/operator.conf \
  stop
```

Expected:

- `actrailctl launch` reports `trace trace-1 entered Active`.
- `actrailviewer payloads` shows `TlsUserSpace` rows with `LIBRARY=openssl`, outbound `SSL_write`, `SSL_write_ex`, or `SSL_write_ex2`, inbound `SSL_read` or `SSL_read_ex`, `Complete`, and operation `success`.
- `actrailviewer events` shows HTTP/2 `Application` rows such as `frame` and `data`.

This preflight covers launch-time `tls-sync` runtime injection, the sync event socket, dynamic OpenSSL probe planning, and daemon-side payload ingestion. Seccomp user notification, `pidfd_open`, `pidfd_getfd`, and `process_vm_readv` are covered by process invocation and socket fallback examples when those paths are enabled.

## Fanotify Enforcement Preflight

```bash
python3 docs/examples/04.fanotify-enforcement-e2e/run_e2e.py
```

Expected:

```text
allowed=ok
denied=permission_denied
```

Those two lines are emitted by the monitored agent process. They prove the allowed file was actually readable and the denied file actually raised `PermissionError`. The same run must also show one allow and one deny `Enforcement` event from AcTrail.

If this fails with `fanotify_init: Operation not permitted`, the environment lacks the required privilege or fanotify permission support. Running inside a restricted container is a common cause.

## OTEL Export Preflight

After any example has produced a completed trace:

```bash
./target/release/actrailviewer export-otel \
  --config <example-operator.conf> \
  --trace-id <TRACE_ID> \
  --output /tmp/actrail-example.otlp.json
```

Expected:

- The output file is pretty JSON with a top-level `resourceSpans` field.
- Examples that produce semantic actions contain spans with `actrail.action.kind`, such as `process.exec`, `file.read`, `file.write`, `file.modify`, `http.message`, `llm.request`, `llm.response`, or `enforcement.decision`.

Some low-level facts currently do not have OTEL spans. For example, raw network transport rows, IPC rows, resource samples, provider labels, and stdio payloads should still be validated through `actrailviewer` views, JSON graph export, or payload commands until they have dedicated semantic export coverage.

## Failure Interpretation

| Symptom | Likely Cause | Action |
| --- | --- | --- |
| `unsupported eBPF target architecture` during build | Target is not `x86_64` or `aarch64` | Use a supported architecture. |
| `kernel BTF is missing` | Kernel does not expose BTF | Install or boot a kernel with BTF enabled. |
| `tracefs mount is missing` or not writable | tracefs not mounted or inaccessible | Mount/enable tracefs according to the host policy. |
| `tracepoint syscalls/sys_enter_dup2 id is unavailable` | Target kernel/architecture omits the compatibility `dup2` tracepoint | Rebuild with a version that treats fd-alias compatibility tracepoints as optional; do not disable socket payload capture. |
| `perf_event_open` permission errors | perf event policy blocks tracepoint attach | Adjust host policy for the test run or run with required capabilities. |
| TLS payload rows missing in `tls-sync` examples | The target was not launched through `actrailctl launch`, the sync runtime was not loaded, the probe plan did not match the target binary, or the daemon did not consume sync payload events | Check `actrailctl launch` stderr and daemon log; do not switch to `track-add` for TLS payload tests. |
| `fanotify_init: Operation not permitted` | Missing fanotify permission support or required capabilities | Run on a host/VM with fanotify permission events and required privileges. |
| OTEL file has no expected span | The example did not produce that semantic action, or current semantic export lacks coverage | Verify the corresponding viewer/event/payload surface and file an export coverage task if the fact should become a span. |
