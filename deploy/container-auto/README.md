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
├── install-host.sh
├── e2e.sh
└── seccomp/
```

## Current Scope

The currently supported and tested deployment target is one Linux host using
Docker:

- `actraild`, storage, viewer, and web run on the host;
- one observed agent container runs `actrailctl launch`;
- host control and TLS-sync Unix sockets are mounted into that container;
- the isolation acceptance case temporarily starts a second container only to
  verify that one container cannot operate on another container's trace.

This change does not claim support for Kubernetes, Podman, direct
containerd/CRI-O operation, multi-host control, TCP socket forwarding, or a
containerized `actraild`.

The final permission matrix and cross-container isolation E2E passed on
x86_64 and ARM64 (Oracle A1) Docker hosts after the daemon-side
permission-resolution rework.

## Selection Matrix

| Host eBPF | Workload seccomp-notify | Immutable profile suffix | Effective coverage |
| --- | --- | --- | --- |
| unavailable/disabled | unavailable/disabled | `ebpf-off-notify-off` | TLS and derived application data |
| unavailable/disabled | available/required | `ebpf-off-notify-on` | TLS plus process exec context (`argv`) |
| available/required | unavailable/disabled | `ebpf-on-notify-off` | TLS, host eBPF system events, and BPF-copy socket payload |
| available/required | available/required | `ebpf-on-notify-on` | Host events, TLS, and process exec context |

The runtime model has no numeric deployment levels; the two permission axes
above are the only selection inputs.

## Automatic Operation

Install the unified host configuration:

```bash
sudo deploy/container-auto/install-host.sh target/release
```

It installs the versioned `/etc/actrail/container-auto.conf`, which declares
the complete capability set and uses host eBPF `enabled = "auto"`.

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
  --security-opt seccomp="$(pwd)/deploy/container-auto/seccomp/actrail-notify.json" \
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

The test starts its own daemon with config, sockets, database, logs, image
context, image tags, and container names under a unique temporary namespace.
Cleanup stops only that daemon and removes only those temporary assets; it does
not install or replace `/etc/actrail`, `/usr/local/bin`, the systemd service, or
an existing AcTrail database.

The same test also starts two isolated workload containers and verifies that
container B cannot list, remove, distinguish the existence of, register a
seccomp listener for, or inject a TLS event into container A's trace.

On Debian/Ubuntu ARM64 builders, the eBPF build automatically adds
`/usr/include/aarch64-linux-gnu` when it contains the target `asm` headers.
Nonstandard sysroots can set `ACTRAIL_BPF_SYSTEM_INCLUDE` explicitly; no
host-level `/usr/include/asm` symlink is required.
