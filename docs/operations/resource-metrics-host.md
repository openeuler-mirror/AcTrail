# Host and workload resource metrics

Resource collection, alerts and storage belong to `resource_profiling`. Host and
workload collection can run together. Their samples use the selected resource
storage backend; the default SQLite database is `actrail-resources.sqlite`, relative
to the operator configuration directory. Trace events and cgroup control records
use the main storage database.

## Collection

For a workload started by `actrailctl launch`, cgroup v2 measures charged memory
for the entire workload hierarchy, including descendants. `memory_current_bytes`
is the kernel's `memory.current`; `memory_peak_bytes` is optional.
`process_rss_sum_kb` measures summed process RSS separately. CPU, I/O and PID
counters accompany memory where supported.

Use a writable delegated cgroup v2 root for the daemon service. The systemd
service needs `Delegate=yes` and `DelegateSubgroup=daemon` (systemd 254+).
Set the root to the service's actual cgroup path:

```toml
[resource_profiling.collection.workload]
enabled = true
mode = "cgroup"
interval_ms = 1000

[resource_profiling.collection.workload.cgroup]
mode = "v2"

[resource_profiling.collection.workload.cgroup.v2.managed]
root = "/sys/fs/cgroup/system.slice/actraild.service"
recovery_scan_limit = 1024
finalization_timeout_ms = 30000

[resource_profiling.alerts]
memory_alert_current_bytes = "1073741824"
```

Workload `auto` permits admission fallback to procfs when a cgroup source cannot
bind the target. Explicit workload `cgroup` mode requires successful binding.
Once moving the stopped launch PID commits admission, later errors fail admission.
Check accounting method, coverage and fallback metadata on samples before
interpreting measurements.

The daemon stays in its own leaf; launches enter trace workload leaves before
exec and descendants inherit membership. After trace completion, sampling waits
for an empty scope and submits a final sample. Control completion is persisted
before removing the empty scope. A timeout produces partial coverage and leaves
surviving descendants running. Durable control records support restart recovery.
If a recovered scope has disappeared, recovery marks the trace terminal and
degraded and records a diagnostic. Failed final counter reads are bounded by the
finalization timeout. Nested empty directories are cleaned with retry after
partial progress.

Host CPU, memory and load use `[resource_profiling.collection.host]`.
Its `enabled` and `interval_ms` settings default to `true` and `1000`. Host samples
are identified by kernel boot ID. Host CPU uses total machine capacity as 100%;
workload CPU uses one logical CPU as 100%. The first sample has no CPU rate until
a baseline exists.

## Storage and queries

```toml
[resource_profiling.storage]
backend = "database"
queue_capacity_batches = 64

[resource_profiling.storage.database]
backend = "sqlite"

[resource_profiling.storage.database.sqlite]
path = "./actrail-resources.sqlite"
write_timeout_ms = 1000

[resource_profiling.storage.database.sqlite.retention]
enabled = true
max_age_seconds = 604800
sweep_interval_seconds = 60
max_samples_per_sweep = 4096
```

The resource SQLite path may point to the same file as the main SQLite storage.
Set both paths explicitly to the same absolute path to share the file. Each
backend owns its tables and retention policy. Separate resource databases are
also supported. The daemon opens configured resource databases before enabling
fanotify and excludes their database, WAL and SHM files from self interception.

Output is best-effort. Bounded queues and write timeouts isolate storage failures
from collection and control completion. Full queues or failed writes may lose
samples, including final samples. Resource retention runs during writes and is
independent of trace retention.

`remote` sends to Prometheus remote-write or InfluxDB line-protocol endpoints.
`hybrid` sends independently to a configured list of database and remote backends.
See the [configuration layout](../architecture/config-layout.md) for each branch's
fields and the [resource configuration guide](../architecture/components/resource-profiling/configuration.md)
for a multi-target storage example.

```bash
actrailviewer --config operator.toml --output-format json host-resources --limit 100
actrailviewer --config operator.toml --output-format json workload-resources --trace-id trace-1 --limit 100
```

Resource commands open the configured resource database read-only.
`--storage-path` supplies a resource SQLite database directly. Hybrid queries
require exactly one database child. Remote-only data is queried through the
receiving service. Web serves workload samples at
`GET /api/traces/{id}/resources?limit=100` on the originating host.

Main storage schema version 43 requires a database created with the current
schema. Existing version 42 databases are rejected at startup; configure a new
database path before starting the current binaries.

## Verification

```bash
cargo build --release -p daemon -p ctl -p view -p web
sudo -E python3 tests/v2/regression/test_all.py --no-profile --case resource_metrics_cgroup
```

The privileged cgroup v2 regression
requires root, cgroup v2 and systemd 254+; it checks hierarchy placement, inherited
membership, completion, restart and timeout. Missing prerequisites are reported as
skips.
