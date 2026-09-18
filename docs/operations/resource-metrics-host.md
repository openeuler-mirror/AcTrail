# Host workload memory accounting

For a workload started by `actrailctl launch`, cgroup v2 measures charged memory
for the entire workload hierarchy, including descendants. `memory_current_bytes`
is the kernel's `memory.current`; `memory_peak_bytes` is optional. Summed process
RSS remains a diagnostic (`process_rss_sum_kb`), not a replacement for cgroup
charges. CPU, I/O and PID counters accompany memory where supported.

## Configuration

Use a writable delegated cgroup v2 root for the daemon service. The systemd
service needs `Delegate=yes` and `DelegateSubgroup=daemon` (systemd 254+).
Set the root to that service's actual cgroup path, for example:

```toml
[resource_metrics]
enabled = true
mode = "cgroup-v2"
cgroup_root = "/sys/fs/cgroup/system.slice/actraild.service"
interval_ms = 1000
finalization_timeout_ms = 30000
orphan_limit = 1024
memory_alert_current_bytes = "1073741824"
```

`procfs` remains the default mode. `auto` permits fallback when managed cgroup
setup is unavailable; `cgroup-v2` requires successful preflight. Once moving the
stopped launch PID commits admission, later errors fail admission rather than
silently switching accounting. Ordinary host attachment has no managed launch
scope and continues to use procfs. Check accounting method, coverage and fallback
metadata on events before interpreting measurements.

The daemon stays in its own leaf; launches enter trace workload leaves before
exec and descendants inherit membership. After trace completion, sampling waits
for an empty scope, persists one final event, and removes the empty scope. A
timeout produces partial coverage and does not kill surviving descendants.
Durable scope records support recovery after daemon restart.

If a recovered scope has disappeared, recovery persists a partial final sample,
marks the trace terminal and degraded, and records a diagnostic. Failed final
counter reads are bounded by the finalization timeout even when the scope is empty.
Nested empty directories are cleaned with retry after partial progress. Archived
databases can be opened read-only without first running a writable migration.

See the [validation record](../designs/resource-metrics-validation.md) and
[review fixes and staged contract fields](../designs/resource-metrics-host-review-fixes.md).

## Verification

```bash
cargo test -p linux_cgroup -p linux_platform -p config_core -p sqlite_storage
cargo test -p daemon -p view -p web
python3 -m unittest tests.v2.regression.resource_metrics_cgroup.test_case
sudo -E python3 tests/v2/regression/test_all.py --no-profile --case resource_metrics_cgroup
```

The privileged regression needs root, cgroup v2, systemd 254+ and release daemon,
CLI and viewer binaries. A prerequisite skip is not acceptance evidence. It tests
hierarchy placement, inherited membership, final events, restart and timeout.
The SQLite tests include frozen DDL from `fb5a6c68` to exercise existing databases.

See the [design](../designs/resource-metrics-cgroup-implementation-plan.md),
[RSS/COW experiment](../designs/resource-metrics-rss-accounting/README.md), and
[three-PR plan](../designs/resource-metrics-pr-split-plan.md). Experiment results
describe the historical revision recorded there, not this rebased branch.
