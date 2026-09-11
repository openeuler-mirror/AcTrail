# RSS/PSS/cgroup memory reproduction

This directory updates the experiment from `r.nuriev/memory-gauging` for the
cgroup-v2 resource accounting implementation. The historical harness used
`track-add`, which still has procfs summed-RSS semantics by design. The updated
harness uses a controlled `actrailctl launch` in a transient systemd service,
allowing AcTrail to place the workload in its own delegated trace cgroup before
`exec`.

The workload reads resource-oracle's staged SWE-bench corpus and Verified task
parquet, forks four pandas/Arrow workers, and repeatedly performs benchmark
aggregations and task-text scans. During the plateau, the harness samples:

- AcTrail `memory_current_bytes` and optional `process_rss_sum_kb` diagnostics;
- the same trace cgroup's `memory.current` and `memory.peak` directly;
- summed `/proc/<pid>/statm` RSS and `/proc/<pid>/smaps_rollup` PSS.

Build and run from the repository root as root:

```bash
cargo build --release -p daemon -p ctl -p view
sudo python3 \
  docs/designs/resource-metrics-rss-accounting/reproduce_swebench_cgroup.py \
  --resource-oracle /home/projects/resource-oracle \
  --output /tmp/actrail-swebench-memory-cgroup.jsonl
```

The script uses a unique `actrail-memory-reproduction-<pid>.service` unit and
removes it after the run. It fails if AcTrail falls back from cgroup v2 or if
the external and AcTrail plateau samples are missing.

## Observed result, 2026-08-27

Tested AcTrail revision: `53203cf` on openEuler 24.03 (LTS-SP1), Linux
`6.18.33.2-microsoft-standard-WSL2`, x86-64.

| Measurement | Plateau samples | Median |
| --- | ---: | ---: |
| AcTrail `memory_current_bytes` | 48 | 154,408 KiB (150.79 MiB) |
| Direct cgroup `memory.current` | 112 | 154,408 KiB (150.79 MiB) |
| AcTrail diagnostic `process_rss_sum_kb` | 48 | 578,168 KiB (564.62 MiB) |
| Independent summed `/proc/*/statm` RSS | 112 | 578,168 KiB (564.62 MiB) |
| Independent summed PSS | 112 | 204,239 KiB (199.45 MiB) |

All 50 periodic events and the single final event used `cgroup_v2`; all had
`exact` coverage. On the five-process plateau, AcTrail's authoritative cgroup
median exactly matched the direct kernel reading. The optional RSS diagnostic
also matched the independent RSS sum, but was 3.74 times the cgroup charge and
2.83 times PSS. This confirms that the updated implementation fixes the metric
semantics without relabeling the old summed-RSS diagnostic.

The compact checked-in provenance and result are in
[`observed-cgroup-v2-summary-openEuler-24.03-2026-08-27.jsonl`](observed-cgroup-v2-summary-openEuler-24.03-2026-08-27.jsonl).

The original synchronized 128 MiB fork/COW control was also repeated on the
same host. Summed RSS moved only from 561,120 to 561,576 KiB (+456 KiB), while
PSS moved from 140,168 to 271,624 KiB (+128.38 MiB) and cgroup
`memory.current` moved from 134,144 to 265,472 KiB (+128.25 MiB). This
reconfirms the premise of the historical experiment: summed process RSS cannot
represent unique charged memory for a fork-heavy workload. Its raw synchronized
measurements are in
[`observed-cow-openEuler-24.03-2026-08-27.jsonl`](observed-cow-openEuler-24.03-2026-08-27.jsonl).
