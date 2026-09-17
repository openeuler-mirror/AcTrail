# Resource metrics: delivery and validation

This is the single validation record for the cgroup series. Configuration and
semantics live in the [host](../operations/resource-metrics-host.md),
[container](../operations/resource-metrics-containers.md) and
[sandbox](../operations/resource-metrics-sandboxes.md) operator guides.

## Delivery boundaries

| PR branch | Review against | Scope |
| --- | --- | --- |
| `r.nuriev/cgroup-host-memory` | `dev` | Managed launches, shared counters, trace lifecycle, storage and consumers |
| `r.nuriev/cgroup-container-memory` | Host branch | Read-only existing-container admission, bindings, sampling and recovery |
| `r.nuriev/cgroup-sandbox-memory` | Container branch | Guest workload identity, collection, negotiated transport and evidence |

Merge into `dev` in that order and rebase successors after each merge (especially
after squash merges). Until then, diffs against `dev` are cumulative. The extra
directories are Git worktrees, not separate projects. Original source is preserved
on `archive/cgroup-memory-source-eca3d6f8`; the saved plans and earlier validation
history remain in Git. The refreshed development base for this split is `499ea6dc`.

## Correctness checks

| Area | Regression evidence |
| --- | --- |
| Managed finalization | Empty scopes with unreadable counters reach a bounded partial final; one failed scope cannot block healthy traces |
| Lost managed scopes | Durable terminal/degraded trace, partial final and diagnostic; restart repair is idempotent |
| Cleanup and peak lifetime | Nested cleanup, partial progress, retry and symlink rejection; production sampler retains its peak reader |
| External admission | Required counters checked before admission; trace/processes/memberships/binding commit atomically; failed write rolls everything back |
| External recovery | Legacy orphan admission repaired with diagnostic; original active trace completes after exit/PID reuse; final event is not duplicated |
| External identity loss | Membership checked before counters; stale transition emits failure while allowing fallback; recovery persists degraded health and one warning |
| Procfs fallback | Saved nonzero start time checked before and after memory reads; reused/unverified PID rejected for live and recovered traces |
| Alerts | Explicit process RSS supports RSS-only thresholds without managed delegation; charged memory is never relabelled RSS |
| Retained state | Forgotten trace barriers/fallback reasons are pruned; live/recovered outcomes share failure/fallback handling |
| Old generated config | Explicit sandbox evidence version 2 accepted and normalized to 3; full generated document validated; unknown versions rejected |
| Guest identity race | Directory replacement between open and read still produces identity and counters from the same descriptor |
| Guest isolation | Missing/unreadable/malformed roots do not suppress healthy workloads; shared boundaries deduplicate roots |
| Guest evidence | Two workloads through production wire codec, async writer, shutdown and reopen; identities, sequence/generation and optional/full-width counters retained |

## Format boundaries

Main SQLite: baseline 26 upgrades to host 27 and container 28. Upstream independently
allocated 27 to HTTP-link role 527; that codebook and index are retained. Read-only
archives do not require new resource registries or a writable upgrade. The frozen
baseline DDL and existing legacy resource tag-6 fixtures remain tested.

Sandbox evidence: independent 2 -> 3 migration; old generated operator settings
normalize to 3 before database initialization, including with workload collection
disabled. Negotiation prevents new workload observations reaching unsupported peers.

Unused PSS fields/method were removed. They were introduced by this unmerged series,
absent from upstream and unsupported by any local release tag; there is no basis
for a deployed-format compatibility claim. Remaining codec values are unchanged
and removed method value 2 is not reassigned. Earlier unmerged development
databases from this series are not supported release fixtures: use fresh test
databases. No user databases are modified by this cleanup.

The unused layout-policy selector, admission storage argument and external
population accessor were removed. The mount-ID parser enters only with the guest
collector that uses it. PSS collection and guest-to-host-trace projection are not
implemented or claimed.

## Reproducible checks

Run from the sandbox branch for cumulative coverage:

```bash
cargo test -p config_core -p model_core -p daemon -p linux_cgroup -p linux_platform -p sqlite_storage -p sandbox_observation -p sandbox_linux_collector -p sandbox_vsock_contract -p sandbox_upstream_contract -p sandbox_evidence_sqlite -p sandbox_agent_runtime -p vsock_gateway_runtime -p sb
cargo check --workspace --lib --bins
python3 -m unittest tests.v2.regression.resource_metrics_container.test_case tests.v2.regression.resource_metrics_cgroup.test_case
```

The container Docker acceptance runner is
`tests/v2/regression/resource_metrics_container/run_e2e.py`. It requires a local
image, never pulls one, and uses disposable workloads and an isolated daemon.
It covers counters, read-only controls, RSS-only alerts, restart sampling, live
removal and original recovered trace closure. On this branch its operator patch
also supplies the baseline explicit sandbox evidence version 2.

Host acceptance is `ResourceMetricsCgroupCase` in
`tests/v2/regression/resource_metrics_cgroup/case.py`. Invoke the case directly
against built development binaries for local checks; the general V2 runner may
install release binaries/system dependencies. It covers exact/restart/timeout,
nested cleanup, lost-scope repair and post-recovery event allocation.

## Latest results and limits

2026-09-08, after the sandbox review fixes:

- Cumulative command above: 127 Rust tests passed; 1 live guest test ignored.
- Python assertion/config smoke tests: 3 passed.
- `cargo check --workspace --lib --bins`: passed.
- Fresh development binaries (`daemon`, `ctl`, `view`): built successfully.
- Docker acceptance with local `openeuler/openeuler:24.03-lts-sp1`: passed,
  including the baseline explicit evidence-schema setting, RSS-only alerts,
  restart and original recovered trace closure.
- Host privileged acceptance: passed exact (18 events), restart (77), timeout
  (27), lost-scope repair (2), nested cleanup and post-recovery trace allocation.
- Diff checks and documentation-link cleanup: passed.

Builds used `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4` and a shared local target directory to
conserve disk space. Disposable acceptance daemons/containers were cleaned up;
no images were pulled or existing workloads used as test targets.

The live guest-counter test remains explicitly ignored here: no configured guest
or KVM environment. Portable pipeline tests do not prove live VM/vsock/mTLS
delivery. Release-binary acceptance is not claimed.

The unrelated upstream full-test-target compilation error remains unchanged:
`semantic_action_runtime/src/live/http_exchange.rs` references
`PendingHttpResponse.received_at`, while the struct defines `observed_at`. This
series does not modify that file or claim a full workspace test pass.

The retained [RSS/COW experiments](resource-metrics-rss-accounting/README.md)
describe their historical source revision, not fresh acceptance of these branches.
