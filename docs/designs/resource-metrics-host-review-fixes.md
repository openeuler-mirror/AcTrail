# Host PR review follow-up

Review baseline: `499ea6dc..482fa988`. All four reported defects were confirmed.

- Empty scopes with failed final reads now retry without aborting the live drain;
  the elapsed deadline independently produces a partial final sample. Missing
  counters remain absent, never zero. The final barrier can complete normally.
- Missing/invalid recovered scopes atomically persist a partial resource final
  event, an orphaned scope record, a degraded terminal trace and a diagnostic.
  Recovery also repairs older orphaned records that have no final event. Repeated
  startup is idempotent. Live event/diagnostic ID seeds are read after recovery.
- Read-only validation accepts baseline 26 and current 27 core schemas without
  requiring the writer-owned scope table or modifying the archive. Writable
  initialization still validates the new registry. Tests use frozen baseline DDL
  and open an actual file read-only, checking that its bytes do not change.
- Cleanup removes directories bottom-up using rmdir, never unlinking controller
  files or following symlinks. It tolerates already-removed directories. Scanning
  recognizes a canonical scope even after its workload leaf has been removed;
  failed startup and finalization cleanup is queued for retry.
- The production sampler retains one cgroup counter reader per trace until final
  persistence. A sampler-level test checks the same peak descriptor across a
  periodic and final read, rather than testing only an unused reader abstraction.

## Deliberately staged contract members

`parse_cgroup2_mount_id` is shared support for PR 3's guest workload identity.
`BroaderThanTrace` is produced by PR 2's whole-container measurements. Unused PSS
fields and accounting method were removed: upstream and local release tags provide
no deployed-format basis for retaining them. Existing assigned methods 0 and 1 are
unchanged; 2 is not reassigned. Databases from earlier unmerged development revisions
of this series are not a supported release format; use fresh test databases.

## Validation scope

The Python contract test is only a configuration smoke test. Lifecycle assurance
comes from Rust regressions and privileged acceptance. The acceptance case now
creates nested empty cgroups and checks cleanup, removes an exact disposable scope
while the daemon is stopped, verifies degraded completion after restart, then
launches another trace to detect recovery/live event-ID collisions.

Validation results are recorded in resource-metrics-host-validation.md. The same
fixes are propagated through the container and sandbox branches; the container
reader accepts archived 26/27 schemas while its writable schema remains 28.
