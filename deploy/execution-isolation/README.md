# Execution-isolation runtime configuration

These files are bounded defaults for the independent observation route:

```text
actrail-sb -> AF_VSOCK -> actrail-vsock-gateway -> TCP -> actraild
```

The SB daemon may be started and snapshotted before Host services exist. To activate a live route,
start `actraild`, start the gateway, restore or start the SB daemon, then issue the SB `connect`
command.

## Files and deployment paths

Install or render the examples to absolute paths owned by the corresponding process:

| Source | Suggested deployed path | Owner |
| --- | --- | --- |
| `actrail-sb.toml` | `/etc/actrail/actrail-sb.toml` | Guest `actrail-sb` |
| `actrail-vsock-gateway.toml` | `/etc/actrail/actrail-vsock-gateway.toml` | Host gateway |
| `actraild-sandbox-resource-alert.startup.toml` | merge into `/etc/actrail/operator.conf` | Host `actraild` |

The startup fragment is not a complete operator configuration. Merge its sections into the
deployment's complete `operator.conf`; do not pass the fragment alone to `actraild`.

The checked-in startup fragment and plugin configuration reference these default deployment
paths. A deployment may use different absolute paths when the fragment and plugin configuration
are updated consistently:

```text
/usr/share/actrail/plugins/sandbox-resource-alert/sandbox-resource-alert.plugin.toml
/usr/share/actrail/plugins/sandbox-resource-alert/sandbox-resource-alert.config.v1.schema.json
/etc/actrail/plugins/sandbox-resource-alert/sandbox-resource-alert.config.json
```

With the checked-in operator configuration, the daemon account must be able to create or write
the independent Sandbox Alert DB at `/var/lib/actrail/sandbox-alerts.sqlite`.

Generate fresh configuration files with the same release binaries that will load them. Without
`--force`, `init` uses create-new semantics and refuses to replace an existing file:

```bash
/usr/bin/actrail-vsock-gateway init \
  --output /etc/actrail/actrail-vsock-gateway.toml \
  --backend firecracker \
  --uds-path /run/firecracker/actrail/vsock.sock \
  --port 43182 \
  --daemon-address 127.0.0.1:9472

/usr/bin/actrail-sb init \
  --output /etc/actrail/actrail-sb.toml \
  --root-process-name xiaoo \
  --root-process-name claude \
  --control-socket /run/actrail/actrail-sb-control.sock \
  --instance-lock-path /run/actrail/actrail-sb.lock
```

Use `--force` only when replacement is intentional. Each generated file is parsed and validated
by its owning binary before `init` returns successfully; parent directories must already exist.

The checked-in TOML files show the same bounded defaults, but automation should use `init` so a
refreshed release remains the source of truth. Start the processes with the generated absolute
configuration paths:

```bash
/usr/bin/actrail-vsock-gateway --config /etc/actrail/actrail-vsock-gateway.toml
/usr/bin/actrail-sb daemon --config /etc/actrail/actrail-sb.toml
/usr/bin/actrail-sb connect \
  --control-socket /run/actrail/actrail-sb-control.sock \
  --host-cid 2 \
  --port 43182
```

The daemon loads and attaches its Guest-only eBPF programs, initializes resource sampling, binds
the Guest-local control socket, and becomes ready without opening VSOCK. Observations collected
before a successful `connect` response are discarded before the bounded sender queue. The connect
command exits successfully only after the daemon has completed the gateway handshake.
The connect command uses the daemon profile's control request deadline and binary frame limit by
default. Deployments may override them with `--request-timeout-ms` and `--max-frame-bytes` when
the daemon's `[control]` profile is changed.

`[sender]` owns transport I/O timeout, reconnect cadence, silence heartbeat interval, batch limit,
and worker stack size. `[control]` owns the Guest-local socket path and mode, request deadline,
pending connection limit, binary frame limit, and control-thread stack size. Host CID and VSOCK
port are runtime activation values and are never written to the daemon TOML.

## Cross-component invariants and checked-in defaults

- The Guest destination port and the Host backend listener endpoint must represent the same
  VSOCK port (the runtime connect example uses `43182`). For Firecracker, Guest
  `actrail-sb connect --port P` corresponds to the Host listener `${uds_path}_${P}`.
- SB `sender.max_silence_interval_ms` must be lower than gateway
  `sb_peer_idle_timeout_ms`, with enough margin for scheduling and I/O jitter. The checked-in
  defaults are 5 seconds and 15 seconds. Observation batches refresh gateway activity; SB emits
  an empty Heartbeat only after the observation path has remained silent for the maximum interval.
- Gateway `upstream.daemon_address` must reach the daemon Hand listener. The checked-in same-host
  defaults use `127.0.0.1:9472` on both sides; a wildcard listener, another interface, or a
  different network namespace does not require identical address strings.
- `outbound_queue_capacity` must be at least
  `max_sb_connections * per_sb_forward_quota`. The checked-in defaults are
  `1024 = 64 * 16`; exact equality is not required.
- Gateway `upstream_heartbeat_interval_ms` must be lower than daemon
  `connection_idle_timeout_ms`, with enough margin for scheduling and I/O jitter. The checked-in
  execution-isolation profile uses 5 seconds and 15 seconds.

The SB example watches Linux `comm` names `xiaoo` and `claude`, each of which fits the kernel's
15-byte name boundary. Change `root_process_names` to the exact executable names used in the
Guest. `require_initial_root = false` permits the observed process to start after the SB daemon;
it does not weaken the eBPF, procfs, or configuration startup checks. VSOCK is established only by
the runtime `connect` command.

## Firecracker endpoint

Firecracker is the checked-in execution-isolation backend. Configure the microVM VSOCK device
with the same base `uds_path` supplied to the gateway. When `actrail-sb` connects to Host CID 2
and destination port `P`, Firecracker forwards the connection to the Host AF_UNIX listener at
`${uds_path}_${P}`. The gateway derives this endpoint; operators do not append the port manually.

The base `uds_path` must be absolute and unique to one Firecracker microVM. A gateway instance
owns one derived Firecracker listener endpoint. Deploy one gateway instance per microVM unless a
higher-level supervisor explicitly owns multiple gateway instances. The connection limit bounds
sessions accepted by that listener; it does not discover other microVM UDS paths.

When Firecracker runs under `jailer`, configure the Host-visible base path after applying the
deployment's mount-namespace and chroot mapping. The gateway does not infer that mapping.

The checked-in gateway section is:

```toml
[vsock]
backlog = 128

[vsock.listener]
backend = "firecracker"
uds_path = "/run/firecracker/actrail/vsock.sock"
port = 43182
```

## Optional backends

Cloud Hypervisor remains available as an independent deployment backend. Render one gateway
configuration per VM with the complete Host listener endpoint owned by that VM lifecycle:

```bash
/usr/bin/actrail-vsock-gateway init \
  --output /run/actrail/<vm-id>/actrail-vsock-gateway.toml \
  --backend cloud-hypervisor \
  --socket-path /run/vc/vm/<vm-id>/clh.sock_43182 \
  --daemon-address 127.0.0.1:9472
```

The generated VSOCK section is equivalent to:

```toml
[vsock]
backlog = 128

[vsock.listener]
backend = "cloud-hypervisor"
socket_path = "/run/vc/vm/<vm-id>/clh.sock_43182"
```

`<vm-id>` is a deployment placeholder and must be resolved before use. `socket_path` must be an
absolute path owned by the VM lifecycle. An existing socket path is not replaced by gateway bind.

Native Host AF_VSOCK is also available for environments that expose a kernel VSOCK listener.
StratoVirt uses this backend: it does not require a StratoVirt-specific gateway transport or a
Unix-socket endpoint rule.

```bash
/usr/bin/actrail-vsock-gateway init \
  --output /etc/actrail/actrail-vsock-gateway.toml \
  --backend native \
  --cid 4294967295 \
  --port 43182 \
  --daemon-address 127.0.0.1:9472
```

Firecracker, Cloud Hypervisor, and StratoVirt through native AF_VSOCK share the same gateway
session and TCP upstream runtime. Backend-specific endpoint resolution remains confined to
gateway startup.
