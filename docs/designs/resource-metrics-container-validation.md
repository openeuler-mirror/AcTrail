# PR 2 validation

Depends on `r.nuriev/cgroup-host-memory`; uses source container portions from
`54eabe16`, `0726243d`, and `eca3d6f8` without guest observations or transport.

- `cargo test -p linux_platform -p config_core -p sqlite_storage -p daemon`:
  passed (57 tests before two additional migration tests).
- `cargo test -p sqlite_storage` after migration additions: passed (18 tests).
- `python3 -m unittest tests.v2.regression.resource_metrics_container.test_case`:
  passed (2 tests, including incorrect accounting/identity/RSS rejection).
- Standalone container acceptance against freshly built development binaries and
  local `openeuler/openeuler:24.03-lts-sp1`: passed. Verified container-scoped
  measurements against kernel memory.current, unchanged controls/membership,
  sampling after daemon SIGKILL/restart, and a single final event on removal.
- `git diff --check`: passed.
- Release-binary acceptance: not run. The same runner accepts `--bin-dir target/release`.

New regression fixtures cover baseline 26, upstream 27 without resource scopes,
host 27 with resource scopes, malformed legacy databases and repeated reopen.
The migration uses version 28 to preserve upstream's independently allocated 27.

The disposable Docker container and isolated acceptance daemon were removed after
the test. No image was pulled and existing workloads were not used as test targets.

## Host review follow-up

Rebased onto the host lifecycle, recovery, cleanup, retained-reader and archived
read-only fixes. Read-only validation accepts 26, 27 and current 28 without
requiring either resource registry; writable validation/migration remains strict.
The inherited archive test exercises both 26 and 27 without a writable upgrade.

## Container review follow-up

See [findings, fixes and regression coverage](resource-metrics-container-review-fixes.md).

- Targeted Rust suites (`daemon`, `sqlite_storage`, `linux_platform`, `linux_cgroup`): passed.
- Python acceptance assertion unit tests: passed (2).
- Fresh development-binary Docker acceptance: passed, including RSS-only alerts
  without managed delegation and original recovered-trace closure/exactly-once finalization.
- `cargo check --workspace --lib --bins`: passed.
- No release-binary acceptance or live sandbox guest run is claimed.
- The unrelated upstream all-target test compilation error concerning
  `PendingHttpResponse.received_at` is unchanged; a full workspace test pass is not claimed.
