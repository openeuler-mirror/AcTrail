# Trace Resource Metrics: cgroup v2 Implementation Plan

> Delivery note: host accounting is PR 1 of the
> [three-PR series](resource-metrics-pr-split-plan.md). The shared filesystem
> implementation now lives in `crates/core/linux_cgroup`, re-exported as
> `linux_platform::cgroup_v2`. See the [operator guide](../operations/resource-metrics-host.md)
> for current configuration and validation commands. The phase descriptions below
> retain the original design rationale; container and guest features are separate PRs.

- Status: implementation-ready after the Phase 0 decisions and acceptance gates below are accepted
- Last source review: 2026-08-26
- Source baseline: `dev` at `fb5a6c6`
- Primary path: Linux cgroup v2, controlled `actrailctl launch`

## Decision summary

AcTrail should make a daemon-owned cgroup v2 subtree the authoritative resource boundary for eligible launched traces. The current sum of per-process RSS values is useful only as a diagnostic: shared pages can be counted repeatedly, short-lived descendants can be missed, and descendants can outlive the root process.

The first implementation slice is deliberately narrow:

- Linux cgroup v2 only.
- `actrailctl launch` workloads whose stopped child can be moved before `exec`.
- Host-managed processes, not processes already owned by a container or VM runtime.
- Trace-wide CPU, memory, I/O, and PID accounting, with a final sample after the cgroup becomes empty or a bounded timeout expires.
- Existing procfs sampling remains available as an explicit fallback and diagnostic mode.

This source review found two changes that materially affect the design:

1. Main event payloads now use a hand-written binary SQLite codec. Extending `ResourcePayload` in place without a new wire tag would make existing rows ambiguous or unreadable. The plan therefore reserves a new resource payload tag and keeps decoding the legacy tag.
2. `agent-sb` now emits guest-wide `GuestResourceSnapshot` observations through the sandbox evidence pipeline. Those readings describe VM/guest pressure, not a trace cgroup. They remain a separate contract and must not be used as the implementation of trace accounting.

## Readiness verdict

The plan is sufficient to begin implementation if the following decisions are treated as fixed:

- `memory.current` is the authoritative current charged memory value. It is not renamed to RSS and is never copied into `rss_kb`.
- `memory.peak`, when the kernel exposes it, is the authoritative peak charged memory value.
- Resource coverage and accounting method are typed fields, not metadata strings.
- A final resource event is a persistence barrier in trace finalization.
- Automatic fallback is allowed only before the stopped child has been successfully moved into the workload cgroup.
- Existing attached processes, containers, and VM workloads are not moved between cgroups in the first slice.
- New regression coverage lives under `tests/v2/`; no new tests are added to legacy test roots.
- The main resource event codec gets a backward-compatible format change before fields are added.

These items are not blockers for Phase 1 and can remain later work:

- Per-subtask cgroups and agent-runtime control operations.
- Exact accounting inside a Kata/VM guest.
- cgroup v1 support.
- Enforcing resource limits rather than observing usage.

## Current implementation and failure mode

The current sampler is `crates/apps/daemon/src/services/resource_metrics.rs`. For each trace it walks active process memberships, reads procfs for every currently known PID, and adds the values together. It emits either `scope=process_tree` or `scope=process`.

This is not a trace-wide memory charge:

- Summed RSS double-counts pages shared by two or more processes.
- A process can fork, consume memory, and exit between sampling intervals.
- Membership discovery can lag process creation and exit.
- The root can exit while descendants continue working.
- PID reuse and incomplete process identity resolution can leave gaps.
- There is no kernel-maintained trace peak or final empty-boundary reading.

The existing fields remain readable for compatibility, but their semantics do not change:

- `rss_kb`: sum of sampled process RSS values only.
- `virtual_memory_kb`: sum of sampled process virtual-memory sizes only.
- `cpu_percent_millis`: existing procfs-derived instantaneous percentage only.

The new cumulative cgroup CPU counters and charged memory values receive distinct fields.

## Boundary with `agent-sb`

There are now two independent resource-observation systems. They must remain explicit in code, storage, and operator output.

| Property | Main trace resource event | `agent-sb` guest resource observation |
| --- | --- | --- |
| Identity | `TraceId` and trace scope | `(gateway_id, sb_id)` and guest boot ID |
| Source | Host cgroup v2 and optional host procfs diagnostics | Guest `/proc/stat`, `/proc/meminfo`, `/proc/vmstat` |
| Memory meaning | Kernel charge to one managed trace cgroup | Guest-wide `MemTotal - MemAvailable` |
| Pipeline | Main event ingest, storage, views, export | Sandbox gateway, sandbox evidence DB/plugins |
| Coverage | Exact only for a daemon-owned trace subtree | Whole-guest pressure; not trace coverage |
| Alerting | Main trace resource policy | `sandbox_resource_alert` categories |

Consequences:

- Do not extend or reuse `sandbox_observation::MemorySnapshot` for the main `ResourcePayload`.
- Do not route trace resource events through the sandbox resource alert plugin.
- Do not label `agent-sb.memory.used_bytes` as cgroup usage, RSS, or a trace peak.
- A host runtime cgroup around a VM or sandbox may be reported as `container`/`sandbox` scope with `BroaderThanTrace` coverage, but it is not exact guest workload accounting.
- Exact per-trace accounting inside a guest is a separate project. It requires guest-side cgroup control and an authenticated mapping from the host trace to a guest workload identity.
- Main resource-metrics changes must leave the sandbox observation wire/storage codecs compatible and covered by targeted non-regression tests.

## Target cgroup hierarchy

Use an explicitly delegated cgroup v2 root. Do not put the daemon in an internal node that has domain controllers enabled.

```text
<delegated-root>/
  daemon/                         # actraild leaf
  traces/                         # empty controller node
    <trace-id>-<random-nonce>/    # empty aggregate node
      workload/                   # root process and ordinary descendants
      subtask-<scope-id>/         # later phase only
```

The aggregate node gives one trace-wide reading while allowing later subtask leaves. It must contain no processes. Enabling controllers follows this order:

1. Verify unified cgroup v2 and delegation ownership.
2. Move `actraild` into `<delegated-root>/daemon` if the service manager did not already place it in a suitable leaf.
3. Enable required controllers in the delegated root and `traces/`.
4. Create the trace aggregate and `workload/` leaf.
5. Enable controllers in the aggregate while it is empty.
6. Move only the stopped launch child into `workload/cgroup.procs`.

Required controllers are `memory` and `cpu`. `io` and `pids` are optional capabilities: missing optional controllers produce absent fields and a diagnostic, not fabricated zeroes. A configured required mode fails preflight if either required controller cannot be delegated.

Never use `cgroup.kill` for routine trace finalization. Observation must not change workload lifetime.

## Resource event contract

Replace stringly typed accounting semantics with typed model fields. Names below are the intended Rust concepts; final spelling should follow existing model conventions.

```rust
enum ResourceAccountingMethod {
    CgroupV2,
    ProcfsRssSum,
    ProcfsPssSum,
}

enum ResourceAccountingCoverage {
    Exact,
    BroaderThanTrace,
    Partial,
}

enum ResourceSampleKind {
    Periodic,
    Final,
}
```

Add these fields to `ResourcePayload`:

- `accounting_method`
- `accounting_coverage`
- `sample_kind`
- `memory_current_bytes`
- `memory_peak_bytes`
- `memory_anon_bytes`
- `memory_file_bytes`
- `memory_swap_current_bytes`
- `memory_events`
- `memory_events_local`
- `cpu_usage_usec`
- `cpu_user_usec`
- `cpu_system_usec`
- `cpu_nr_throttled`
- `cpu_throttled_usec`
- `io_read_bytes`
- `io_write_bytes`
- `pids_current`
- `pids_peak`
- `process_rss_sum_kb`
- `process_pss_sum_kb`

Use typed counter structs for `memory.events` and `memory.events.local` with at least `low`, `high`, `max`, `oom`, `oom_kill`, and `oom_group_kill` where the kernel exposes them. Unknown future keys may be preserved in metadata but must not cause the sample to fail.

Contract rules:

- Optional files and unavailable counters map to `None`; zero is used only when the kernel returned zero.
- `rss_kb` is populated only from `process_rss_sum_kb` for backward compatibility.
- `memory.current` and `memory.peak` are bytes, never KiB.
- Cgroup CPU counters are cumulative. Do not put them in `cpu_percent_millis`.
- A derived CPU rate may continue to populate `cpu_percent_millis`, but its interval and basis must be documented and it must be absent for the first sample when no prior point exists.
- `scope=trace` is used for the daemon-owned aggregate.
- `scope=process` and `scope=process_tree` retain current procfs meanings.
- `scope=container` or `scope=sandbox` is allowed only with `BroaderThanTrace` unless AcTrail created an exclusive boundary for the trace.
- `Exact` means all processes charged to the managed subtree and no unrelated processes. It does not claim that `memory.current` equals physical RSS.

The metadata map remains for diagnostics such as cgroup relative path, sampled process count, missing optional files, and fallback reason. It is not the canonical home of numeric counters or accounting semantics.

## SQLite event-codec compatibility

`crates/storage/adapters/sqlite/src/records/event_codec/manual.rs` currently assigns tag `6` to the legacy resource layout and rejects trailing bytes. Appending fields before or after its metadata map would break either old or new decoding.

Use this migration strategy:

1. Keep tag `6` as `ResourceLegacy` on decode.
2. Allocate the next unused event-payload tag, currently `11`, as `ResourceV2`.
3. Make the encoder emit tag `11` for the expanded `ResourcePayload`.
4. Decode tag `6` into the expanded model using:
   - `accounting_method=ProcfsRssSum`
   - `accounting_coverage=Partial`
   - `sample_kind=Periodic`
   - legacy CPU/RSS/virtual-memory fields unchanged
   - all new cgroup counters absent
5. Decode tag `11` with the complete new layout.
6. Keep the database event variant string as `resource`; this is a payload-codec revision, not a new public event kind.

Before changing the encoder, add:

- A frozen byte fixture for at least one tag-6 resource payload.
- A test proving that fixture still decodes after the model expansion.
- Tag-11 round-trip tests with all fields, absent fields, and unknown metadata keys.
- A database-level test that reads a pre-change resource row.
- A corrupt/truncated tag-11 test and the existing trailing-byte rejection.

JSON serialization also needs defaults or an explicit compatibility adapter for historical resource records. Avoid a global `#[serde(default)]` that silently accepts malformed new records; constrain defaults to the legacy decode path or versioned JSON contract.

## Platform component

Add a narrow cgroup v2 adapter under `crates/core/linux_platform` rather than embedding filesystem logic in the daemon service. It should expose typed operations for:

- discovering the current unified hierarchy and the daemon's relative cgroup;
- validating delegation and controller availability;
- creating and removing the managed hierarchy safely;
- enabling subtree controllers;
- creating a trace aggregate/workload leaf using a validated `TraceId` plus random nonce;
- moving a PID by writing `cgroup.procs`;
- verifying membership by reading `/proc/<pid>/cgroup` and the leaf;
- reading aggregate counters;
- reading `cgroup.events` and its `populated` value;
- recursively checking whether a managed subtree is empty;
- removing empty managed directories from leaf to aggregate;
- scanning only the configured managed root for orphan recovery.

Path construction must reject separators, `..`, empty identifiers, and paths outside the configured root. Use directory-relative operations where practical to reduce time-of-check/time-of-use risk. Unit tests use a temporary fake cgroup filesystem; privileged kernel tests are separate V2 regressions.

Counter parsing requirements:

- tolerate whitespace and reordered key/value lines;
- reject duplicate known keys and invalid integers;
- saturate or diagnose aggregation overflow instead of wrapping;
- treat a file disappearing during teardown as a race to retry or report, not a daemon panic;
- distinguish unsupported files from permission errors and malformed data.

## Configuration and preflight

Extend the existing resource-metrics configuration without changing its default behavior in the first compatibility release:

```toml
[resource_metrics]
enabled = true
mode = "auto"                 # procfs | cgroup-v2 | auto
interval_ms = 1000
include_children = true
include_system = true
cgroup_root = "/sys/fs/cgroup/actrail"
finalization_timeout_ms = 30000
orphan_limit = 1024
cpu_alert_percent_millis = 80000
memory_alert_rss_kb = 1048576
```

Compatibility rules:

- Existing fields keep their meanings.
- `procfs` reproduces current behavior.
- `cgroup-v2` is required mode: daemon startup/preflight fails if the hierarchy cannot be used safely.
- `auto` selects cgroup accounting for an eligible controlled launch and otherwise emits an explicit procfs fallback reason.
- The rollout may initially retain `procfs` as the default, switching to `auto` only after V2 acceptance and upgrade tests pass.

Preflight must report:

- cgroup filesystem version;
- resolved managed root and delegation owner;
- available/enabled required and optional controllers;
- whether the no-internal-process rule is satisfied;
- ability to create, enable, read, and remove a disposable child subtree;
- service-manager guidance, including `Delegate=yes` for systemd;
- whether configured mode will fail, operate exactly, or fall back.

Do not silently fall back in required `cgroup-v2` mode.

## Controlled-launch admission

The existing launch protocol already creates a stopped pre-exec child and sends its pidfd. Integrate cgroup admission at that boundary.

A launch is eligible when all are true:

- resource metrics are enabled;
- the negotiated sensor plan requests `Capability::ResourceMetrics`;
- the request uses controlled launch mode;
- the process is host-managed rather than already assigned to a container/VM runtime cgroup;
- configuration and preflight allow cgroup v2 accounting.

Admission sequence:

1. `actrailctl` creates the child in its current stopped state and sends PID plus pidfd.
2. The daemon resolves and validates process identity as it does today.
3. The daemon creates the trace aggregate and workload leaf.
4. The daemon writes the stopped PID to `workload/cgroup.procs`.
5. It verifies the same live process through pidfd/identity checks and confirms cgroup membership.
6. It persists the trace and durable resource-scope relation.
7. It returns `TrackAdded`; only then does the client release the child to `exec`.

The successful migration in step 4 is the commit boundary:

- Before it, `auto` may abandon the new subtree and use procfs.
- After it, verification or admission failure rejects `TrackAdd`; the client terminates the still-stopped child through the existing launch failure path, and the daemon queues safe asynchronous subtree cleanup.
- Do not attempt to move the process back to an inferred parent cgroup. That is unsafe without a separately designed rollback protocol.

Repeated requests must be idempotent by request/trace identity. A retry must not create a second active subtree or move a process twice.

## Durable resource-scope registry

Add a small SQLite registry rather than relying only on reconstructing directory names:

```text
trace_resource_scopes(
  trace_id,
  nonce,
  relative_path,
  accounting_method,
  lifecycle_state,
  created_at,
  final_event_id,
  updated_at
)
```

The exact schema can follow storage conventions, but it must support these invariants:

- At most one live daemon-owned scope per trace.
- The relation is durable before `TrackAdded` is acknowledged.
- Final event persistence and the registry's `finalized` transition are atomic or idempotently recoverable.
- Startup recovery scans only the configured root and reconciles registry rows with directories.
- Unknown directories are quarantined/diagnosed and removed only after validating that they are empty and match the managed naming contract.
- A bounded orphan count prevents unbounded work or attacker-controlled directory traversal.

If the daemon crashes before trace/scope persistence, an empty or later-empty directory can be cleaned as an orphan but no trace resource event is invented. If the relation was persisted, recovery resumes sampling/finalization and avoids duplicate final events.

## Sampling

Refactor the sampler around an accounting source selected per trace:

```text
ResourceAccountingSource
  - CgroupV2(managed trace scope)
  - Procfs(process membership set)
  - ExternalCgroup(read-only broader scope; later/explicit only)
```

For a managed trace, read counters from the aggregate node so future subtask leaves roll up automatically. A periodic sample is valid while the root is alive, while descendants remain, and while the trace is draining.

Sampling behavior:

- A single missing optional controller does not discard the whole event.
- Failure of a required cgroup read emits a diagnostic and marks trace resource health degraded; it must not silently relabel a procfs reading as exact.
- Procfs RSS/PSS diagnostics may be gathered at a lower frequency because `/proc/*/smaps_rollup` can be expensive.
- A cgroup sample can still include `process_rss_sum_kb` for comparison when configured, but the cgroup values remain authoritative.
- Alerts use the field appropriate to their configured policy. The existing `memory_alert_rss_kb` continues to evaluate RSS only; add a separately named cgroup-memory threshold rather than changing its meaning.
- System-wide host metrics remain metadata/host diagnostics and are not trace charges.

## Finalization and trace lifecycle

Root exit is not the resource boundary. The trace resource boundary ends when the aggregate reports `populated=0` or when the configured finalization deadline expires.

Add a non-blocking resource finalization state machine:

```text
Sampling
  -> WaitingForEmpty
  -> ReadingFinal
  -> PersistingFinal
  -> Finalized
  -> Cleaning
```

On root removal or entry into `Draining`:

1. Stop treating root exit as the last resource sample.
2. Continue periodic reads while `cgroup.events: populated=1`.
3. When it becomes zero, read all counters one final time.
4. Persist one `sample_kind=Final` event.
5. Mark the durable resource scope finalized in the same transaction or through an idempotent event ID.
6. Remove empty leaf/aggregate directories.
7. Release the trace to the existing semantic/post-trace finalizer.

On timeout:

- Persist a final event with `accounting_coverage=Partial` and the timeout reason.
- Mark trace health degraded.
- Retain the scope in the orphan registry for bounded background cleanup.
- Do not kill or move remaining processes.
- Continue the existing shutdown/finalization path so a hostile descendant cannot retain the daemon forever.

### Required `TraceRuntime` change

The current lifecycle can leave a trace in `Draining` while memberships remain open, but the existing terminal finalizer only queues terminal traces and treats open memberships as a blocker. Resource finalization therefore cannot merely plug into the existing terminal queue.

Implement these explicit semantics:

- The resource finalization queue accepts `Draining` traces/root-removal notifications independently of the terminal queue.
- After the exact-empty or timeout final event is durably persisted, call a new `TraceRuntime` transition such as `complete_after_resource_barrier(trace_id, finished_at)`.
- The transition permits `Draining -> Completed`; it does not synthesize process exits or erase membership evidence.
- If memberships are still open, preserve them, mark trace health degraded, and emit a diagnostic such as `resource_finalized_with_open_memberships`.
- An empty cgroup can still give `Exact` resource coverage even when process-lifecycle evidence is incomplete. Resource coverage and trace health are separate claims.
- Persist the updated trace state before enqueueing the existing terminal semantic/post-trace work.
- The current terminal finalizer must check that the resource barrier is persisted for managed cgroup traces. Procfs traces have no empty-cgroup barrier and follow their documented partial final-sample path.

During daemon shutdown, add resource draining as a bounded stage within the existing global shutdown deadline. Do not replace the current staged shutdown or let per-trace deadlines multiply the global deadline.

## Attach, containers, and sandboxed workloads

Phase 1 must be conservative:

- Never move an already-running attached process. Moving a PID changes accounting/limits and may violate its service manager or container runtime.
- If an operator explicitly selects an existing cgroup for read-only observation, validate authorization and report `BroaderThanTrace` unless exclusivity is proven.
- Otherwise attached traces use procfs with `Partial` coverage.
- Container/runtime cgroups are read-only and broader by default. Do not create children inside a runtime-owned hierarchy.
- A VM or Kata host cgroup accounts for the VM/runtime boundary, not an individual guest trace. Label it `sandbox` or `container` with `BroaderThanTrace`.
- `agent-sb` guest-wide readings can be correlated in a UI later, but correlation does not turn them into main trace resource events.

cgroup v1 behavior is explicit: required mode fails; `auto` falls back to procfs with a reason.

## Later: subtask scopes

The hierarchy intentionally supports later per-subtask leaves, but Phase 1 must not expose a half-implemented public API.

A future control operation must:

- be authenticated and authorized through every UDS/control layer;
- receive a pidfd or equivalent non-reusable process identity, not trust a raw PID;
- let the daemon allocate a validated scope ID and create `subtask-<scope-id>`;
- move only a stopped or otherwise safely coordinated process;
- be idempotent across retry;
- define parent/child accounting and aggregation semantics;
- integrate with the external agent runtime only after its launch hook and identity contract are available.

Until then, all launched descendants inherit `workload/`, and the aggregate still produces correct trace-wide accounting.

## Consumer updates

Audit every exhaustive `ResourcePayload` construction/rendering site. At minimum the current tree includes:

- daemon sampling and resource alerts;
- `crates/storage/adapters/sqlite/src/records/event_codec/`;
- viewer JSON and text rendering;
- web event and topology views;
- JSON graph export attributes;
- ingest policy-gate tests/fixtures;
- eBPF database verification, which currently expects legacy RSS and virtual-memory fields;
- model serde tests and any event fixtures.

Presentation rules:

- Display accounting method and coverage beside the values.
- Label `memory_current_bytes` as cgroup charged memory, not RSS.
- Preserve old resource rows and their legacy labels.
- Do not make cgroup-only events fail verification because `rss_kb` or `virtual_memory_kb` is absent.
- Export typed numeric values rather than duplicating them only as metadata strings.

## Verification strategy

All new maintained regressions go under:

```text
tests/v2/regression/resource_metrics_cgroup/
tests/v2/common/resource_metrics/        # shared helpers only when useful
```

Unit tests:

- cgroup path validation and safe hierarchy construction;
- controller parsing and enablement decisions;
- every counter-file parser, including reordered/unknown/duplicate keys;
- absent optional files and malformed required files;
- accounting method/coverage/sample-kind serialization;
- tag-6 legacy codec fixtures and tag-11 round trips;
- lifecycle state-machine races and idempotent finalization;
- timeout, daemon restart, and orphan reconciliation;
- alert-field selection without changing legacy RSS threshold semantics.

Privileged Linux V2 regressions:

- preflight succeeds on a delegated cgroup v2 root and cleans its disposable subtree;
- a release `actrailctl launch` child is already in `workload/` before `exec`;
- child and grandchildren are charged to the trace aggregate;
- shared-memory/process-tree workload demonstrates why RSS sum differs from cgroup charge;
- a short-lived memory spike is visible in `memory.peak` even between polling intervals;
- root exits while a descendant continues, and finalization waits for empty;
- final sample is persisted exactly once before post-trace completion;
- timeout produces partial coverage without killing the descendant;
- required mode fails and auto mode reports fallback when delegation is unavailable;
- daemon restart reconciles a persisted scope and an unregistered empty orphan;
- no-internal-process hierarchy rules remain satisfied;
- optional `io`/`pids` absence does not fabricate values.

The existing controlled OOM assets under `tests/v2/common/execution_isolation/` may be reused or generalized when their semantics fit. Production behavior remains cgroup v2 even if a shared test helper can detect v1.

`agent-sb` non-regression:

- its guest resource contract and fixed codec round-trip unchanged;
- gateway/sandbox storage still accepts old observations;
- sandbox resource alerts still use guest CPU and available-memory semantics;
- no main trace ID or cgroup field is added to the guest snapshot as part of this work.

Tests requiring cgroup delegation must detect the prerequisite and report a clear skip in unsupported developer environments. CI must have at least one required privileged lane where a skip is a failure. Acceptance uses release binaries.

## Implementation sequence

### Phase 0: contracts and compatibility

1. Add frozen legacy resource-codec fixtures.
2. Add typed accounting enums/counters and expanded `ResourcePayload`.
3. Add ResourceV2 codec tag and backward decoder.
4. Update consumers to tolerate cgroup-only fields and legacy records.
5. Add configuration parsing/validation without changing default runtime selection.

Exit gate: existing databases open, legacy resource events render, and all main plus sandbox codec tests pass.

### Phase 1: platform and preflight

1. Implement the cgroup v2 adapter and fake-filesystem unit tests.
2. Add hierarchy/delegation preflight and systemd guidance.
3. Add the durable resource-scope registry and startup reconciliation primitives.
4. Add the privileged preflight regression.

Exit gate: AcTrail can safely create, validate, read, persist, recover, and remove an empty managed trace subtree.

### Phase 2: controlled launch and sampling

1. Insert admission into the stopped-child `launch` path.
2. Enforce the migration commit boundary and idempotency.
3. Select cgroup versus procfs source per trace.
4. Emit periodic typed cgroup samples.
5. Add release-binary launch, descendants, shared-memory, and peak regressions.

Exit gate: eligible launches report exact trace-wide periodic metrics without changing attach/container behavior.

### Phase 3: finalization and recovery

1. Add the independent resource finalization state machine.
2. Add the explicit `Draining -> Completed` resource-barrier transition.
3. Make final event and registry state persistence recoverable/idempotent.
4. Integrate the bounded shutdown stage and background orphan cleanup.
5. Add root-exit, timeout, restart, and exactly-once regressions.

Exit gate: every managed trace either has one exact final event after empty or one explicitly partial final event after timeout, and existing post-trace work begins only after that event is durable.

### Phase 4: rollout

1. Run upgrade tests against representative pre-change SQLite databases.
2. Run V2 privileged tests across supported kernels/systemd deployments.
3. Compare cgroup charge, process RSS sum, and workload-known allocations to validate labels rather than force equality.
4. Switch the default from `procfs` to `auto` only after the compatibility and operational gates pass.

## Definition of done

The first production slice is complete when:

- an eligible controlled launch is placed in a daemon-owned leaf before `exec`;
- its aggregate cgroup remains authoritative after root exit and across descendants;
- periodic and final events carry typed method, coverage, and sample kind;
- memory charge and RSS are never conflated;
- a final event is durably ordered before terminal semantic/post-trace processing;
- timeouts degrade explicitly without killing workloads;
- legacy tag-6 resource rows remain readable;
- attach, container, VM, and `agent-sb` paths retain their explicit non-exact or independent semantics;
- release-binary V2 regressions pass in a required delegated-cgroup CI lane.

At that point implementation can proceed to optional PSS diagnostics, read-only external-cgroup observation, and subtask scopes without reopening the core accounting or lifecycle contract.
