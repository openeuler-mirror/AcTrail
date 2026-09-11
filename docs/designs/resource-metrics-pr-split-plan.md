# Resource metrics: three-PR implementation plan

## References and delivery

- Source: `fb5a6c68..eca3d6f8`, preserved as `archive/cgroup-memory-source-eca3d6f8`.
- Refreshed target on 2026-09-07: `upstream/dev` at `499ea6dc` (12 commits newer).
- `origin` is the contributor fork; its `dev` branch no longer exists.
- Local `dev` tracks `upstream/dev`.
- Branches: `r.nuriev/cgroup-host-memory`, `r.nuriev/cgroup-container-memory`,
  `r.nuriev/cgroup-sandbox-memory`.

Implement as a dependent series, with every stage buildable. Publish PRs targeting
`dev` sequentially after predecessors merge, rebasing dependent branches as needed.
Opening all three against unchanged dev would show cumulative diffs. The original
source branch remains untouched. No remote PR creation is part of this local split.

## PR 1: host-launched workload accounting

Outcome: controlled launch enters a delegated cgroup before exec; resource samples
and a final measurement survive trace finalization and daemon restart.

1. Apply host changes from `a87d1355` and `53203cf7` onto refreshed dev, resolving
   conflicts without reverting upstream features.
2. Introduce the shared `linux_cgroup` crate at its final location and retain the
   `linux_platform::cgroup_v2` re-export. Bring forward common reader fixes/tests.
3. Include managed scope admission, sampling, recovery, cleanup, configuration,
   resource event fields, scope persistence, codecs, exports and UI consumers.
4. Inspect later sampler fixes and include those needed for host correctness.
5. Keep container policy/admission and sandbox observations out of this PR.
6. Include host design, an operator guide, the existing V2 host regression and
   RSS/COW reproduction material (historical evidence, not a new validation run).

Validation: counter/path/parser tests, config and codecs, scope persistence,
launch admission, finalization/restart/timeout coverage, affected crate tests,
and the privileged host acceptance case when prerequisites are available.
Exercise an actual baseline schema rather than a fixture generated from new DDL.

## PR 2: existing container accounting on the host

Outcome: attach resolves the existing runtime-owned container boundary and samples
it without moving processes or writing runtime-owned cgroup controls.

1. Start on completed PR 1. Extract host-container portions of `54eabe16`,
   `0726243d`, and `eca3d6f8`.
2. Include container identity models and layout resolution, external binding
   lifecycle/storage/retention, read-only runtime, attachment and finalization.
3. Include disabled/prefer/require policy and stale-binding/fallback behavior.
4. Place container-related procfs runtime recognition fixes here.
5. Split host-container sections out of the combined design document. Explain
   whole-container coverage, identity checks, configuration and limitations.
6. Include existing unit tests and add dedicated container acceptance coverage.

Validation: supported/unsupported layouts, PID identity/movement, boundary inode
replacement/disappearance, policies, restart recovery, final event, migrations,
and agreement with direct kernel measurements without cgroup mutations.

## PR 3: sandbox workload cgroups

Outcome: agent-sb collects guest workload counters and transports observations
through the gateway into sandbox evidence storage.

1. Start on completed PR 2. Extract remaining guest changes from `0726243d`
   and `eca3d6f8`.
2. Include workload identities/contracts, collector/procfs checks, bootstrap,
   configuration, workers, wire codecs, capability negotiation and reconnects.
3. Include sandbox evidence migration and affected observation consumers.
4. Write a sandbox operator/design guide for implemented behavior. Keep guest
   memory distinct from host VMM accounting; identify authenticated guest-to-trace
   projection as future work rather than claiming it is implemented.
5. Include contract/collector/transport/storage tests and dedicated guest-path
   acceptance coverage with explicit prerequisites.

Validation: workload boundary/identity/deduplication, counters, malformed payloads,
wire round trips, capability downgrade/reconnect, evidence migration and storage,
and guest counters compared with persisted observations when a guest is available.

## Shared-file boundaries and final audit

Split config, daemon resource_metrics/live/attach/shutdown, Cargo files and SQLite
schema/backend/writer changes by feature. Do not use whole-file replacement where
upstream changed the same file. Give each PR coherent implementation and validation
commits, with its own documentation and tests.

Run formatting/diff checks and affected tests at each stage. Record commands and
results, distinguishing passed checks from skipped/unavailable privileged checks.
Do not call an unexecuted acceptance test passed. Compare the series against a
source-on-current-dev integration reference; explain deliberate differences for
docs, tests, compatibility fixes and upstream conflict resolution.

## Progress

- Planning: saved before feature edits.
- Remote refresh: complete; local dev updated to upstream/dev at 499ea6dc.
- Branch creation: all three local feature branches created.
- PR 1: implemented; affected Rust tests, Python contract test and privileged
  development-binary acceptance passed. Release-binary acceptance not run.
- Integration finding: upstream already allocated schema 27 to HTTP links;
  baseline 26 upgrade belongs in PR 1, external bindings use schema 28 in PR 2.
- PR 2 and PR 3: separate branches created; implementation in progress.
