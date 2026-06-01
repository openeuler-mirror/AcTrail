# AcTrail Performance Benchmarks

This directory contains deterministic local benchmarks for estimating AcTrail runtime overhead.

## What It Measures

- `file`: repeated local file open/read/write/rename/truncate/unlink plus one shared mmap write.
- `process`: repeated local helper process execution.
- `http`: repeated loopback HTTP/1.1 POST requests.
- `agent`: a combined local agent-like loop: file I/O, periodic helper execution, and loopback HTTP requests.
- `claude-code`: real external Claude Code CLI request.
- `opencode`: real external opencode CLI request.

The benchmark compares these modes:

- `baseline`: workload only, no AcTrail daemon.
- `daemon-idle`: daemon running, workload not attached.
- `observed-ebpf-core`: `actrailctl launch` with eBPF process/file/network observation.
- `observed-ebpf-payload`: `observed-ebpf-core` plus stdio payload, socket plaintext payload, HTTP/1.1 semantics, and resource sampling. It does not enable seccomp exec/fork/clone or TLS LLM payload capture.
- `observed-seccomp-agent`: process/network observation plus seccomp exec/fork/clone observation and agent invocation semantics. This mode is intended for real external LLM CLI wall-clock tests and is not a superset of `observed-ebpf-payload`.

## Run

Build release binaries first:

```bash
cargo build --release
```

Run all cases and all modes:

```bash
python3 tests/performance/run_benchmark.py \
  --case all \
  --mode all \
  --output local/performance-benchmark.md
```

Run one case:

```bash
python3 tests/performance/run_benchmark.py \
  --case agent \
  --mode baseline,observed-ebpf-payload \
  --repetitions 30 \
  --output local/performance-agent.md
```

All benchmark constants are in `benchmark.conf`: iteration counts, payload sizes, timeouts, binary paths, operator configs, and output path. Do not change the Python code to tune workload size.

The default `file.operations` value is intentionally moderate. Raising it turns the case into an event-ingest stress test; that is useful, but it should be reported separately from the normal end-to-end overhead number.

The `claude-code` and `opencode` cases require external network access and a pre-authenticated CLI. Their timings include provider/network latency, so use them to understand real agent wall-clock impact, not to isolate collector overhead.

## Result

The Markdown report contains:

- median and p95 target task runtime per case/mode,
- target task runtime overhead percentage versus each case's baseline median,
- outer command wall-clock as a separate diagnostic table,
- distribution-level overhead assessment using all baseline and observed target task samples,
- a two-sample KS p-value for same-distribution evidence,
- a Hodges-Lehmann pairwise ratio overhead estimate with bootstrap confidence interval,
- a one-sided Mann-Whitney p-value after shifting baseline by the configured overhead threshold,
- raw per-run timings,
- the traced `trace-N` id for observed runs.

The threshold, alpha, bootstrap count, permutation count, and random seed are configured in `benchmark.conf` under `[statistics]`.
The default threshold question is distribution-level: are observed samples credibly more than 5% slower than baseline samples?
The benchmark does not pair iteration N in baseline with iteration N in observed mode, because real CLI and local OS scheduling noise do not give those positions a stable matched meaning.
The 5% assessment uses `task_runtime_ms`, measured by `tests/performance/lib/timing_wrapper.py` directly around the target workload or CLI command in both baseline and observed modes.
`outer_wall_ms` is reported only to show wrapper/control-plane cost such as `actrailctl launch`, trace registration, and daemon communication.

Treat a report as invalid if any run fails, times out, or the script reports a degraded trace for any reason other than the current `actrailctl launch` root `BootstrapGap` diagnostic.
The script validates only diagnostics whose `trace_id` matches the observed run; daemon-global diagnostics are not used to fail a specific benchmark trace.
