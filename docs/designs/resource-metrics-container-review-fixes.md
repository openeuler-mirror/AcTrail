# Container review follow-up

Review baseline: `499ea6dc..e596cf6e`. All five correctness findings were confirmed.

1. Attachment prepares an in-memory reader, checks required counters, then atomically
   persists the trace, processes, memberships and external binding. A storage failure
   rolls back the whole admission. Legacy orphan bindings are discarded with a
   durable warning in the same transaction, without inventing a trace or final event.
2. Recovered active traces now reconcile root identity on every finalization poll.
   Exit, zombie state, PID reuse and boot changes provide a completion path. The
   persisted terminal lifecycle precedes the atomic final-event/binding close, so a
   crash between these operations is recoverable on the next startup.
3. Periodic reads verify process identity and membership before consuming retained
   counters. Verification failure immediately persists stale state and enters the
   shared fallback disposition.
4. Admission requires readable, parseable `memory.current` and `cpu.stat`. `require`
   rejects unusable sources; `prefer` reports an admission fallback without a binding.
5. Configured RSS alerts use explicit process RSS from read-only subtree enumeration,
   independently of `memory.current`, with no managed-hierarchy dependency.

## Scope and cleanup

- Removed unused `remove_directory` and guest-only platform parsing.
- Moved `parse_cgroup2_mount_id` out of this PR; only the sandbox collector needs it.
- Removed PSS fields at the host foundation after finding no deployed-format
  evidence for the earlier compatibility claim. No assigned remaining values were
  changed. Earlier unmerged development databases require fresh test databases.
- Retention prunes finalized-barrier and fallback-reason caches when traces are
  forgotten. External finalization also releases fallback state.
- Live and recovered sampling share success, failure, diagnostic-deduplication and
  stale/fallback disposition. Membership selection remains specific to live runtime
  versus recovered storage records.

## Regression coverage

- Production admission persistence: binding-write failure rolls back trace, process
  records and memberships; retry commits successfully.
- Production external runtime: missing required counters reject admission, prepared
  admission has no durable binding, valid counters sample, movement becomes stale.
- Repeated startup with a legacy orphan succeeds and emits one durable diagnostic.
- A recovered active trace stays open while identity matches and becomes terminal
  after PID identity changes, including when its binding was already stale.
- Forgotten trace caches are pruned.
- Docker acceptance: required container admission without managed delegation,
  counter comparison, RSS-only alerts, unchanged controls/membership, restart
  sampling, live removal, original recovered trace closure and no duplicate final
  event after another restart.

Validation results are recorded in [PR 2 validation](resource-metrics-container-validation.md).

Subsequent sandbox review also fixed shared fallback identity validation (including
a second start-time check after memory reads) and stale-source degradation. Direct
identity loss emits a failure while allowing fallback; recovery persists degraded
health and an idempotent warning. Removed the ignored layout-policy argument and
unused admission storage argument and population accessor.
