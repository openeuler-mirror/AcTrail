# Host container cgroup design (PR 2)

This records the boundary and lifecycle design extracted from the original combined
plan. See [current operation and validation](../operations/resource-metrics-containers.md)
for the implemented configuration and migration version. Guest collection and
unimplemented guest-to-trace projection are outside this PR.

## Strict host layout policy

The host parser has no policy selector: a generic hexadecimal-leaf fallback is
guest-oriented and unsafe as host proof.

Host proof accepts only known structure with an exact 64-character lowercase hexadecimal ID:

| Runtime | Accepted host forms |
| --- | --- |
| Docker | `docker-<64hex>.scope`, `/docker/<64hex>` |
| containerd | `cri-containerd-<64hex>.scope`, verified Kubernetes pod/container layout |
| CRI-O | `crio-<64hex>.scope`, verified CRI-O Kubernetes layout |
| Podman | `libpod-<64hex>.scope`, `/libpod/<64hex>` |
| Unknown Kubernetes CRI | verified pod ancestor plus exact 64-hex leaf |

Add `Podman` and `Crio` to `ContainerRuntime`. Persist runtime and full normalized ID. Prefix IDs and arbitrary hex leaves are insufficient.

Guest collection belongs to PR 3. Its collector handles:

```text
/default/<64hex>
/k8s.io/<64hex>
```

No unused guest parser is shipped in the host platform adapter. Host fixtures prove
that these guest-only layouts are rejected.

## Boundary identity and ancestry

Return:

```rust
pub struct ContainerCgroupIdentity {
    pub runtime: ContainerRuntime,
    pub container_id: NormalizedContainerId,
    pub pod_uid: Option<String>,
    pub unified_process_path: PathBuf,
    pub container_boundary_path: PathBuf,
}
```

`NormalizedContainerId` contains 32 decoded bytes and renders as 64 lowercase hexadecimal characters.

Prove component-by-component that the selected boundary is an ancestor of actual membership. String-prefix checks are forbidden. A process may be in a descendant such as:

```text
/system.slice/docker-<id>.scope/app.service
```

The measurement must read the verified container boundary.

## PID-reuse and movement protocol

Expected non-zero `HostProcessCoordinates.start_time_ticks` is mandatory.

Binding sequence:

1. Read `/proc/<pid>/stat`; require the expected start time.
2. Read `/proc/<pid>/cgroup`.
3. Parse under `HostRuntime` policy.
4. Open the boundary beneath the cgroup2 mount.
5. Read `/proc/<pid>/stat` again; require the same start time.
6. Read membership again; require it remains below the boundary.
7. Stat the open boundary and record its identity.

A membership change restarts once from step 1; a second change fails with `membership_unstable`. Any start-time change is PID reuse.

During sampling:

- movement inside the verified boundary is allowed;
- movement outside immediately makes the binding stale with `root_moved`;
- AcTrail never automatically binds the same trace to a replacement boundary.

## Read-only adapter

Do not weaken `ManagedCgroupV2`. Refactor into:

```text
CgroupV2CounterReader  -> verified handle, parsing only
ManagedCgroupV2        -> create/move/enable/remove/finalize
ExternalCgroupV2       -> resolve/read only
```

`ExternalCgroupV2` exposes no write, move, controller, limit, kill, reset, or removal API.

Resolve relative to an open cgroup2 mount with `openat2` and:

```text
RESOLVE_BENEATH
RESOLVE_NO_SYMLINKS
RESOLVE_NO_MAGICLINKS
```

Record relative path, `st_dev`, and `st_ino`. Open counters with `O_RDONLY | O_CLOEXEC`. Keep a read-only `memory.peak` descriptor for the binding lifetime when practical. Never open it writable.

Missing optional counters produce `None`; required read failures remain errors and never produce zero.

## Configuration truth table

Add:

```toml
[resource_metrics]
mode = "procfs"
existing_container_cgroups = "disabled" # disabled | prefer | require
external_cgroup_failure_threshold = 3
memory_alert_current_bytes = "disabled"   # or a positive integer string
```

Repository-compatible default:

```text
mode=procfs, existing_container_cgroups=disabled
```

Future recommended configuration after rollout:

```text
mode=auto, existing_container_cgroups=prefer
```

| Mode | Existing setting | Managed host launch | Existing container |
| --- | --- | --- | --- |
| `procfs` | `disabled` | procfs | procfs |
| `procfs` | `prefer`/`require` | configuration error | configuration error |
| `auto` | `disabled` | managed when available, else procfs | procfs |
| `auto` | `prefer` | managed when available, else procfs | external when valid, else procfs |
| `auto` | `require` | existing auto semantics | external required; attachment fails if unavailable |
| `cgroup-v2` | `disabled` | managed hierarchy required | procfs, honoring explicit disablement |
| `cgroup-v2` | `prefer` | managed hierarchy required | external when valid, else procfs with diagnostic |
| `cgroup-v2` | `require` | managed hierarchy required | external required; attachment fails if unavailable |

`require` governs admission. Losing the source later degrades the trace; it never terminates the workload.

Preflight reports managed writable delegation and external read-only capability separately.

## Durable lifecycle and schema

Do not overload `TraceResourceScope`.

```rust
pub enum ExternalBindingState { Active, Stale, Closed }

pub enum ExternalBindingStaleReason {
    HostBootChanged,
    BoundaryMissing,
    BoundaryIdentityChanged,
    ContainerIdentityMismatch,
    RootMoved,
    RequiredCounterUnavailable,
    PermissionDenied,
    RepeatedReadFailure,
}
```

Persist:

```sql
CREATE TABLE trace_external_cgroup_bindings (
    trace_id             INTEGER PRIMARY KEY,
    runtime              TEXT NOT NULL,
    container_id         TEXT NOT NULL CHECK (length(container_id) = 64),
    relative_path        TEXT NOT NULL,
    cgroup_device_be     BLOB NOT NULL CHECK (length(cgroup_device_be) = 8),
    cgroup_inode_be      BLOB NOT NULL CHECK (length(cgroup_inode_be) = 8),
    host_boot_id         BLOB NOT NULL CHECK (length(host_boot_id) = 16),
    lifecycle_state      TEXT NOT NULL CHECK (
        lifecycle_state IN ('active', 'stale', 'closed')
    ),
    stale_reason         TEXT,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    last_good_at         INTEGER,
    closed_at            INTEGER,
    final_event_id       INTEGER,
    CHECK (
        (lifecycle_state = 'active' AND stale_reason IS NULL
         AND closed_at IS NULL AND final_event_id IS NULL)
        OR
        (lifecycle_state = 'stale' AND stale_reason IS NOT NULL
         AND closed_at IS NULL AND final_event_id IS NULL)
        OR
        (lifecycle_state = 'closed'
         AND closed_at IS NOT NULL AND final_event_id IS NOT NULL)
    )
);
```

Device/inode use unsigned big-endian 8-byte blobs because `u64` may exceed SQLite's signed range.

Storage trait operations:

```text
create_external_cgroup_binding
get_external_cgroup_binding
list_live_external_cgroup_bindings
record_external_cgroup_success
record_external_cgroup_failure
mark_external_cgroup_stale
append_final_event_and_close_external_binding
```

The final operation is one transaction. Retry after commit returns the stored event identity rather than inserting a duplicate.

### Main database migration

The refreshed upstream schema is version 27. This PR adds a transactional upgrade
from baseline 26 or upstream/host 27 to version 28:

1. Create the table and lifecycle index.
2. Retain upstream semantic-action codes, including HTTP-link role 527.
3. Validate old and new objects.
4. Set `PRAGMA user_version=28` immediately before commit.

Add a frozen v26 database fixture and rollback test.

### State machine

```text
Active -> Stale
Active -> Closed
Stale  -> Closed
Closed -> no transition
```

There is no automatic `Stale -> Active`.

Identity mismatch, root movement, boot mismatch, boundary deletion/replacement, container mismatch, or permission denial causes immediate Stale. Retryable I/O leaves Active with a gap; reaching `external_cgroup_failure_threshold` causes `Stale(RepeatedReadFailure)`.

Once stale, start a distinct procfs series next interval with `Partial` coverage and `fallback_from=external_container_cgroup`. Never relabel procfs as cgroup. Mark trace health degraded.

### Recovery and retention

On startup, verify boot ID, path, device, inode, runtime, and full container ID for Active rows. Failed rows become Stale with a reason. Stale live traces resume procfs. Closed rows never reopen.

Closed rows remain until normal trace retention deletes them in the trace cleanup transaction.

## Host sampler, alerts, and finalization

Use:

```rust
enum HostTraceResourceSource {
    Managed(ManagedTraceSource),
    ExternalContainer(ExternalContainerSource),
    Procfs,
}
```

Guest series never appear in this enum.

External payload:

```rust
ResourcePayload {
    scope: "container".into(),
    subject: format!("container:{container_id}"),
    measurement_location: ResourceMeasurementLocation::Host,
    accounting_method: ResourceAccountingMethod::CgroupV2,
    accounting_coverage: ResourceAccountingCoverage::BroaderThanTrace,
    memory_current_bytes: Some(counters.memory_current_bytes),
    memory_peak_bytes: counters.memory_peak_bytes,
    ..ResourcePayload::default()
}
```

Metadata includes ownership, runtime, full ID, relative path, `memory_peak_window=cgroup_lifetime`, and attachment time. Never copy `memory.current` into `rss_kb`.

CPU baseline and alert state are keyed by `ResourceSeriesKey`, not only `TraceId`.

Keep `memory_alert_rss_kb` RSS-only. `memory_alert_current_bytes` applies only to cgroup current memory. Alerts include location, scope, subject, coverage, and threshold name.

External cgroups never use the managed empty barrier. At trace terminalization:

1. Active binding: attempt a fresh final read.
2. Stale binding: create a partial final marker with stale reason and last-good time, without replaying old counters.
3. Atomically insert the final event and transition to Closed.
4. Drop in-memory state only after commit.

A crash before commit retries; a crash after commit sees Closed and does not duplicate. Never wait for an external container to become empty.
