# PR 1 validation

Base: upstream/dev `499ea6dc`. Source: `a87d1355`, `53203cf7`, and shared
reader/charged-memory-alert portions of the later source commits.

- `cargo test -p linux_cgroup -p linux_platform -p config_core -p sqlite_storage --lib`:
  passed (33 tests before additional migration/alert tests).
- `cargo test -p daemon -p view -p web -p config_core -p sqlite_storage`:
  passed (40 tests, plus empty binary/doc-test targets), with debug information
  and incremental compilation disabled to conserve disk space.
- `python3 -m unittest tests.v2.regression.resource_metrics_cgroup.test_case`: passed.
- `git diff --check`: passed.
- Privileged development-binary acceptance: passed on systemd 255 and cgroup v2.
  Exact case: 19 events, 78,622,720-byte peak; restart: 77 events; timeout:
  27 events. Invoked the case directly against this branch's `target/debug`
  binaries to avoid the general runner's system-wide release installation.
- Release-binary acceptance: not run; this worktree has no release binaries.

Upstream already uses schema 27 for additive HTTP link role 527. This PR retains
that codebook and index and adds a transactional upgrade from the frozen baseline
26 schema. The container PR must use schema 28 rather than collide with upstream.

The historical RSS/COW experiment is included as supporting documentation only.

## Review follow-up

See resource-metrics-host-review-fixes.md for the confirmed issues and changes.
Expanded privileged development-binary acceptance passed: nested cleanup in the
exact case (18 events, 78,352,384-byte peak), restart (77 events), timeout
(27 events), lost-scope recovery (2 events) and a new trace after recovery.
Focused Rust tests cover empty/unreadable finalization, independent trace progress,
reader retention in the production sampler, cleanup retries/symlink rejection,
durable lost-scope repair/idempotence and read-only archive compatibility.
