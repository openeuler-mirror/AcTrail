# Container Permission Auto-Selection

AcTrail selects a container deployment from two independent permission axes:

```text
--host-ebpf auto|required|disabled
--seccomp-notify auto|required|disabled
```

`host-ebpf` describes whether the running host daemon exposes its eBPF
collector. `seccomp-notify` describes whether the workload container permits
AcTrail's seccomp user-notify launch path, including `pidfd_getfd`.

This directory is the self-contained deployment bundle:

```text
deploy/container-auto/
├── README.md
├── container-auto.conf
├── Dockerfile
├── actraild.service
├── deploy.sh
├── install-host.sh
├── render-otel-http-config.sh
├── wait-service-active.sh
├── e2e.sh
└── seccomp/
```

## Current Scope

The currently supported and tested deployment target is one Linux host using
Docker:

- `actraild`, storage, viewer, and web run on the host;
- one or more observed agent containers each run their own
  `actrailctl launch`;
- the same host control and TLS-sync Unix sockets are mounted into every
  observed container;
- concurrent traces from different container PID namespaces are collected by
  the same host eBPF instance and remain independently attributable;
- the isolation acceptance cases verify both concurrent collection and that
  one container cannot operate on another container's trace.

This change does not claim support for Kubernetes, Podman, direct
containerd/CRI-O operation, multi-host control, TCP socket forwarding, or a
containerized `actraild`.

The final permission matrix and cross-container isolation E2E passed on
x86_64 and ARM64 (Oracle A1) Docker hosts after the daemon-side
permission-resolution rework.

## Multiple Containers on One Host

Run exactly one host `actraild` for a control-socket path and mount its socket
directory into every workload container:

```text
host actraild
├── /run/actrail/control.sock
├── /run/actrail/tls-sync.sock
├── container A -> actrailctl launch -> trace A
└── container B -> actrailctl launch -> trace B
```

The socket paths are shared listeners, not per-container files. Accepted
connections are authenticated with kernel `SO_PEERCRED`, and each trace is
bound to the creating process's PID and mount namespaces. Runtime container
IDs are best-effort attribution only and do not participate in authorization.
Consequently, sharing the mounts does not merge traces and does not let a
container in another PID namespace control or inject TLS-sync data into the
trace.

The eBPF collector uses host PID/TID values for internal map keys, so identical
container-local PIDs from different PID namespaces do not collide. Each trace
also stores its own PID namespace identity; emitted events retain
container-local PID coordinates for the viewer while host PID coordinates
remain available for attribution.

This demo is tested with Docker's default independent PID and mount
namespaces. Host/shared-PID workloads remain separated when their mount
namespaces differ. Workloads that deliberately share both namespaces are
outside the authorization model and require a future explicit workload
capability/runtime binding.

Every workload must invoke `actrailctl launch` for the agent root process.
Mounting the sockets alone does not automatically trace every process in a
container. Concurrent capacity is bounded by `[control].active_trace_max`,
`[control].pending_connection_max`, and the configured eBPF process/pending
map sizes. A second daemon cannot bind the same Unix socket paths.

## Selection Matrix

| Host eBPF | Workload seccomp-notify | Immutable profile suffix | Effective coverage |
| --- | --- | --- | --- |
| unavailable/disabled | unavailable/disabled | `ebpf-off-notify-off` | TLS and derived application data |
| unavailable/disabled | available/required | `ebpf-off-notify-on` | TLS plus process exec context (`argv`) |
| available/required | unavailable/disabled | `ebpf-on-notify-off` | TLS, host eBPF system events, and BPF-copy socket payload |
| available/required | available/required | `ebpf-on-notify-on` | Host events, TLS, and process exec context |

The runtime model has no numeric deployment levels; the two permission axes
above are the only selection inputs.

## Prerequisites

`deploy.sh` builds and installs; it does not provision the host. Install these
first, then the deployment runs unattended:

| Requirement | Why | Installed by the scripts? |
| --- | --- | --- |
| Docker, with a running daemon | Builds the workload image and runs observed containers. `deploy.sh` checks `docker info` and stops if the daemon is unreachable. | No |
| systemd | `install-host.sh` installs and starts `actraild.service`. | No |
| Rust toolchain >= 1.90 | Builds the release binaries. `install-build-deps.sh` checks the version but never installs Rust. | No — checked only |
| `awk grep install mktemp seq sleep systemctl` | Used directly by `deploy.sh`. | No — present on any normal distribution |
| clang, llvm, libelf, zlib, pkg-config, OpenSSL headers, musl toolchain | Builds the eBPF collector and the musl TLS-sync runtime. | Yes — `scripts/install-build-deps.sh` via `dnf` or `apt-get` |
| Node.js >= 18 and npm | Builds the actrailweb frontend. | Yes — same script |
| root | Installs into `/usr/local/bin`, `/etc/actrail` and `/etc/systemd/system`. | No — run with `sudo -E` |

Kernel-side requirements for eBPF, BTF, tracefs, seccomp and fanotify are a
separate matter; see [../../docs/platform-requirements.md](../../docs/platform-requirements.md).
The two permission axes degrade explicitly when the kernel cannot provide them,
so a host that fails those checks still deploys and still reports what it lost.

A host that already has the release binaries can skip the whole build
toolchain by passing `--bin-dir`, in which case only Docker, systemd and root
are required.

## Automatic Operation

Deploy the host daemon and build the workload image in one command. The default
is openEuler 24.03:

```bash
sudo -E deploy/container-auto/deploy.sh
```

The command builds and installs the current AcTrail release, pulls
`openeuler/openeuler:24.03-lts-sp3` when missing, builds
`actrail/container-auto:openeuler-24.03`, installs the versioned host config,
plugins, seccomp profile and systemd unit, starts `actraild`, then smoke-tests
the image and service with a real `actrailctl launch -- /bin/true` trace. Reuse an already-built release with
`--bin-dir target/release`.

Ubuntu 24.04 is the supported alternative:

```bash
sudo -E deploy/container-auto/deploy.sh --distro ubuntu
```

Use `--base-image` for an approved mirror, `--image` for a custom output tag,
and `--pull-policy missing|always|never` to control registry access. Resolve all
defaults without changing the host with `deploy.sh --print-plan`.

By default the host keeps live semantic spans in
`/var/lib/actrail/export/live-spans.otlp.jsonl`. To additionally send them to
an OTLP/HTTP Collector, pass the endpoint as seen by the **host** `actraild`:

```bash
sudo -E deploy/container-auto/deploy.sh \
  --otel-endpoint http://127.0.0.1:4318/v1/traces
```

This installs and loads `otel-http` alongside the local `otel-jsonl` exporter.
The deployment succeeds only after its observed smoke trace increments
`metric.otel_http.successful_batches`; an active plugin with an unreachable
Collector is reported as a failure. Host loopback is valid here because the
daemon runs on the host, unlike a Kata Guest where loopback names the Guest.

The safe default is `--otel-attribute-mode metadata-only`: command lines and
HTTP/LLM content attributes stay local. A trusted Collector and transport may
opt in with `--otel-attribute-mode full`. This permits semantic action content
attributes but does not export raw SQLite `payload_segments` as OTLP spans.

The lower-level host-only installer remains available:

```bash
sudo deploy/container-auto/install-host.sh target/release
```

The host-only equivalent with a Collector is:

```bash
sudo deploy/container-auto/install-host.sh \
  --otel-endpoint http://127.0.0.1:4318/v1/traces \
  target/release
```

It installs `/etc/actrail/container-auto.conf`, the `otel-jsonl` package,
`/etc/actrail/seccomp/actrail-notify.json`, and the systemd service. The
operator config declares the complete capability set, uses host eBPF
`enabled = "auto"`, and loads the exporter through the plugin startup
lifecycle.

Probe from the workload:

```bash
actrailctl --config /etc/actrail/container-auto.conf probe \
  --host-ebpf auto \
  --seccomp-notify auto
```

Launch:

```bash
actrailctl --config /etc/actrail/container-auto.conf launch \
  --host-ebpf auto \
  --seccomp-notify auto \
  -- command args...
```

To make seccomp-notify available while retaining Docker's outer seccomp
allowlist, start the workload container with the versioned profile:

```bash
docker run \
  --security-opt seccomp=/etc/actrail/seccomp/actrail-notify.json \
  -v /run/actrail:/run/actrail:ro \
  -v /etc/actrail:/etc/actrail:ro \
  actrail/container-auto:openeuler-24.03 \
  ...
```

For a trusted test environment or compatibility diagnosis, Docker's outer
seccomp filter can instead be disabled explicitly:

```bash
docker run --security-opt seccomp=unconfined ...
```

This also makes the AcTrail seccomp-notify path available, but removes Docker's
outer syscall filtering. It does not disable the seccomp-notify filter that
`actrailctl` installs for the launched agent. Use the versioned profile for
normal deployments; use `seccomp=unconfined` only when the broader syscall
surface is intentional.

Human output reports both the requested and effective permissions:

```text
deployment_permissions_requested=host_ebpf:auto,seccomp_notify:auto
deployment_permissions_selected=host_ebpf:enabled,seccomp_notify:disabled
deployment_permissions_degraded=true
```

Seccomp-notify status comes from a local launch probe inside the workload
container. The ctl sends that result to the daemon before spawning the
workload. The daemon combines it with its own host eBPF collector status and
operator config, then returns the final immutable profile and effective launch
switches. `--skip-daemon` is only a local probe preview; launch always requires
the daemon decision.

## Fixed Permission Contracts

Use `required` when losing a permission must stop the workload:

```bash
# Requires complete host observation and process exec context.
actrailctl ... launch \
  --host-ebpf required \
  --seccomp-notify required \
  -- command
```

Use `disabled` to guarantee AcTrail does not use that mechanism even when it is
available:

```bash
# TLS-only, with neither host eBPF nor seccomp-notify bound to the trace.
actrailctl ... launch \
  --host-ebpf disabled \
  --seccomp-notify disabled \
  -- command
```

The daemon still runs as root when host eBPF is disabled because process and
container attribution, peer authentication, and host-owned state directories
remain host responsibilities.

## Acceptance Test

Run the complete matrix acceptance test with:

```bash
sudo BIN_DIR=target/release deploy/container-auto/e2e.sh
```

The acceptance image defaults to `openeuler/openeuler:24.03-lts-sp3`. Select
Ubuntu explicitly with:

```bash
sudo CONTAINER_AUTO_E2E_BASE_IMAGE=ubuntu:24.04 \
  BIN_DIR=target/release deploy/container-auto/e2e.sh
```

When the public registry is unavailable, pull the selected distribution image
through an approved mirror and set:

```bash
sudo CONTAINER_AUTO_E2E_BASE_IMAGE=<mirror-image> \
  BIN_DIR=target/release deploy/container-auto/e2e.sh
```

The test starts its own daemon with config, sockets, database, logs, image
context, image tags, and container names under a unique temporary namespace.
Cleanup stops only that daemon and removes only those temporary assets; it does
not install or replace `/etc/actrail`, `/usr/local/bin`, the systemd service, or
an existing AcTrail database.

The same test also starts two isolated workload containers and verifies that
container B cannot list, remove, distinguish the existence of, register a
seccomp listener for, or inject a TLS event into container A's trace.

Run the real-agent concurrent-collection acceptance case with:

```bash
sudo python3 tests/agent-trace/multi-container-xiaoo/run_e2e.py
```

It runs two real xiaoO processes in separate Docker PID namespaces, holds both
traces Active concurrently, and requires independent eBPF process/network
evidence, task-specific `file.read`/`file.write` actions, and successful
`llm.call`, `llm.request`, and `llm.response` actions. The two containers use
different trace names and tasks; the second starts 10 seconds after the first
while retaining an overlapping Active window. On a trusted legacy Docker/runc
host that cannot load the current versioned seccomp profile, add
`--seccomp-profile unconfined` only for compatibility testing.

On Debian/Ubuntu ARM64 builders, the eBPF build automatically adds
`/usr/include/aarch64-linux-gnu` when it contains the target `asm` headers.
Nonstandard sysroots can set `ACTRAIL_BPF_SYSTEM_INCLUDE` explicitly; no
host-level `/usr/include/asm` symlink is required.
