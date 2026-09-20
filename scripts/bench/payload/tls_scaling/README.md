# TLS fork and exec scaling

```sh
python3 -m scripts.bench.payload.tls_scaling.run --out local/bench/payload/ts
python3 -m scripts.bench.payload.tls_scaling.summary --out local/bench/payload/ts
```

Use one previously built release (`--bin-dir` overrides `target/release`). The
runner invokes the existing payload harness with N=1000/2000/3000, one warmup,
three measured rounds, both fork and exec, and one integer iteration. It does
not build Rust. The harness compiles its native workload driver as usual.

Profiles are bare and matched P tls-sync/bpf-copy. Both observed profiles set
the same 65535-byte segment/operation limits; only `capture_backend` differs.
Each job refreshes defaults through the existing harness. Resolved profiles are
compared after replacing job-specific isolation paths and the backend value.
N=1000 and 3000 run sync then direct; N=2000 reverses the backend order. Bare runs
once per N inside the sync job. There are 18 warmups and 54 measurements.

Exec uses fork, exec of the driver's own file, and waitpid, serially. Pure fork
children exit directly. Completion output verifies N successful children;
database event counts verify collection coverage, not exact exec counts. These
are native process diagnostics without TLS application traffic or an agent
conversation. The repeated child file has the same inode within a job; separate
jobs compile separate files and require identical driver hashes.

Task CPU is wait4 user+system including reaped descendants. Observed task CPU
also includes ctl launch. Daemon CPU extends through trace finalization and
excludes daemon startup/shutdown. External daemon identities are recorded by
the harness, and only each job's isolated daemon is stopped. Sample validity is
not inferred from concurrency or wall time alone.

`manifest.json` records actual commands and binary/driver hashes. Each job saves
raw samples, defaults/configuration snapshots, workload stdout, and its SQLite
database. The summary checks all expected rounds, successful workloads, clean
finalized traces, process-event coverage, unchanged drivers, and matched
resolved configuration. It reports CPU/wall means and ranges, direct-versus-sync
and relative-to-bare differences, and adjacent N growth in ms/additional child.
Percentage denominators are the stated before backend; zero baselines have no
percentage. Historical data are not mixed into the result.
