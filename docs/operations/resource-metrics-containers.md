# Existing container memory accounting

Attach to an existing container process to read its runtime-owned cgroup from
the host. AcTrail validates the full container ID, PID start time, cgroup path,
boot ID and directory identity. Sampling never moves container processes or
changes runtime-owned limits or controllers.

```toml
[resource_metrics]
mode = "auto"
existing_container_cgroups = "prefer"
external_cgroup_failure_threshold = 3
memory_alert_current_bytes = "1073741824"
```

`disabled` keeps existing container accounting off. `prefer` falls back to procfs
if admission cannot verify the boundary; `require` rejects that attachment.
Combining prefer/require with `mode = "procfs"` is invalid. The default remains
procfs/disabled. `cgroup-v2` mode also requires managed host delegation, even when
the requested attachment is to a container. `auto` can use read-only container
accounting without managed host delegation. Alert settings accept a positive
integer string or `"disabled"`; zero is not a disable value.

Docker, containerd, Podman, CRI-O and recognized Kubernetes cgroup layouts require
full 64-character lowercase hexadecimal IDs. Arbitrary hexadecimal leaves and
guest-only `/default/<id>` layouts are not host container proof.

Container samples use scope `container`, method `cgroup_v2`, and coverage
`broader_than_trace`: every process charged to that boundary contributes, even
when absent from the trace. The peak covers the container cgroup lifetime, not
just the attachment interval. RSS is not inferred from `memory.current`.
When `memory_alert_rss_kb` is configured, a separate read-only enumeration of the
container subtree samples process RSS. These measurements populate `rss_kb` and
`process_rss_sum_kb` and evaluate the RSS threshold independently of charged-memory
alerts, including when no managed host hierarchy is available.

Bindings are durable and revalidated on restart. Identity loss or repeated read
failure makes a binding stale; fallback is labelled. Require applies at admission,
not as a policy to kill a workload after later counter failure. Final events and
binding closure are stored atomically; retention removes the associated registry.
Identity loss degrades trace health and persists a warning, including on startup.
Procfs fallback verifies saved process start times before and after memory reads;
a reused PID cannot contribute metrics to the old trace.
Main SQLite schema 28 upgrades baseline 26, upstream 27 and host-PR 27 databases
and preserves upstream HTTP-link role 527 and its index.

Admission reads required counters before accepting a source, then commits the
trace, process records, memberships and binding in one transaction. Startup repairs
legacy bindings without a trace by discarding only those invalid admission rows and
persisting a warning diagnostic; it does not fabricate trace or resource events.
Each periodic sample verifies PID start time and container membership before
reading counters. Recovered active traces reconcile their root process identity;
exit (including zombie state), PID reuse or a changed boot completes the trace with
degraded health, followed by an idempotent final event and binding closure.

## Verification

```bash
cargo test -p linux_platform -p config_core -p sqlite_storage -p daemon
python3 -m unittest tests.v2.regression.resource_metrics_container.test_case
python3 tests/v2/regression/resource_metrics_container/run_e2e.py --bin-dir target/release --image YOUR_LOCAL_IMAGE
```

The standalone acceptance runner uses a local Docker image with `/bin/sh` and
`sleep`, starts a disposable container and isolated daemon, attaches twice across
a daemon restart, reads events through the viewer, and checks accounting against
kernel counters plus unchanged cgroup controls. It never pulls an image. Root,
cgroup v2, access to Docker and built daemon/CLI/viewer binaries are required.
Missing prerequisites are reported as a skip (exit 77), not a pass.
The restart test stops the container and requires the **original recovered trace**
to complete with exactly one final event, including after a second daemon restart.
It also verifies RSS-only alerts and live explicit-removal finalization.
See the [validation record](../designs/resource-metrics-validation.md) for results
and format limitations.
