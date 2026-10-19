# Existing container memory accounting

Attach to an existing container process to read its runtime-owned cgroup from
the host. AcTrail validates the full container ID, PID start time, cgroup path,
boot ID and directory identity. Sampling never moves container processes or
changes runtime-owned limits or controllers.

```toml
[resource_profiling.collection.workload]
mode = "auto"

[resource_profiling.collection.workload.cgroup]
mode = "v2"

[resource_profiling.collection.workload.cgroup.v2.external]
policy = "prefer"
failure_threshold = 3

[resource_profiling.alerts]
memory_alert_current_bytes = "1073741824"
```

With `cgroup.mode=v2`, `external.policy=disabled` disables external v2 attachment.
`prefer` permits admission fallback; `require` rejects an unverifiable attachment.
The cgroup table is interpreted in workload `auto` and `cgroup` modes.
Workload `auto` can select procfs when the chosen cgroup version cannot bind the
target; workload `cgroup` requires a cgroup binding. Read-only external v2
attachment works independently of managed host delegation. Alert settings accept
a positive integer string or `"disabled"`.

Docker, containerd, Podman, CRI-O and recognized Kubernetes cgroup layouts require
full 64-character lowercase hexadecimal IDs. Arbitrary hexadecimal leaves and
guest-only `/default/<id>` layouts are not host container proof.

Container samples use scope `container`, method `cgroup_v2`, and coverage
`broader_than_trace`: every process charged to that boundary contributes, even
when absent from the trace. The peak covers the container cgroup lifetime, not
just the attachment interval. RSS is not inferred from `memory.current`.
When `resource_profiling.collection.workload.cgroup.v2.process_rss.enabled = true`, a separate read-only enumeration of the
container subtree samples process RSS. These measurements populate `rss_kb` and
`process_rss_sum_kb` and evaluate the RSS threshold independently of charged-memory
alerts, including when no managed host hierarchy is available.

Bindings are durable and revalidated on restart. Identity loss or repeated read
failure makes a binding stale and suspends that source. Admission fallback is labelled. Require applies at admission,
not as a policy to kill a workload after later counter failure. Final samples are submitted through best-effort resource storage.
Binding closure is persisted independently in control storage; trace retention
removes the associated registry.
Identity loss degrades trace health and persists a warning, including on startup.
Procfs fallback verifies saved process start times before and after memory reads;
a reused PID cannot contribute metrics to the old trace.

Admission reads required counters before accepting a source, then commits the
trace, process records, memberships and binding in one transaction. Startup repairs
legacy bindings without a trace by discarding only those invalid admission rows and
persisting a warning diagnostic; it does not fabricate trace or resource events.
Each periodic sample verifies PID start time and container membership before
reading counters. Recovered active traces reconcile their root process identity;
exit (including zombie state), PID reuse or a changed boot completes the trace with
degraded health, followed by final sampling and idempotent binding closure.

## Verification

```bash
cargo build --release -p daemon -p ctl -p view
python3 tests/v2/regression/resource_metrics_container/run_e2e.py --bin-dir target/release --image YOUR_LOCAL_IMAGE
```

The standalone acceptance runner uses a local Docker image with `/bin/sh` and
`sleep`, starts a disposable container and isolated daemon, attaches twice across
a daemon restart, reads resource samples through the viewer, and checks accounting against
kernel counters plus unchanged cgroup controls. It never pulls an image. Root,
cgroup v2, access to Docker and built daemon/CLI/viewer binaries are required.
Missing prerequisites are reported as a skip (exit 77), not a pass.
The restart test stops the container and requires the **original recovered trace**
to complete and close its binding, including after a second daemon restart.
With a healthy local resource database, it also checks one final sample.
It also verifies RSS-only alerts and live explicit-removal finalization.
See [resource storage and queries](resource-metrics-host.md#storage-and-queries)
for output configuration and best-effort behavior.

## Cgroup v1

```toml
[resource_profiling.collection.workload]
mode = "cgroup"

[resource_profiling.collection.workload.cgroup]
mode = "v1"

[resource_profiling.collection.workload.cgroup.v1]
binding_attempts = 2
allow_hierarchy_root = false
```

The v1 backend binds existing cpuacct and memory controller directories separately.
It reads counters without moving processes, changing limits or creating scopes.
Samples use `cgroup_v1` and `broader_than_trace`; shared groups include processes
outside the trace. CPU uses `cpuacct.usage` nanosecond deltas. Memory uses charged
`memory.usage_in_bytes`, with hierarchical `memory.stat` details and lifetime peak;
these values do not populate process RSS. Restart restores the saved boundaries,
resets the CPU rate baseline and retains the selected version. Changed boundary
identity stops periodic sampling; finalization records unavailable counters as partial.

```bash
python3 tests/process/resource-profiling/v1_e2e.py --out tmp/resource-v1-e2e --image YOUR_LOCAL_IMAGE
```

This runner uses refreshed defaults, a real xiaoo executable and a local streaming
LLM provider inside a disposable container. It compares persisted samples with
kernel counters, verifies unchanged controls and membership, restarts the daemon,
and checks explicit removal and recovered completion with a healthy local resource
database. Binding closure and final sample output are independent.
It requires Docker, readable v1 cpuacct/memory controllers and a local image with
Python 3 plus the runtime libraries needed by xiaoo.
