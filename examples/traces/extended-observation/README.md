# Extended Observation Example Trace

This directory contains a checked-in JSON graph export from a live AcTrail run.
It is meant as a compact fixture for viewer, documentation, and integration
work that needs a trace with more than one observation domain.

## Files

- `generation.conf`: configuration used by the live generator.
- `viewer.conf`: configuration used by `actrailviewer` when exporting this trace.
- `trace.json`: exported graph document for `trace-1`.

## Included Observations

The exported graph contains:

- process lifecycle spans: fork, exec, signal, exit
- file spans: open, read, write, mmap_shared, mkdir, rmdir, rename, unlink,
  truncate
- IPC spans: pipe, FIFO, Unix socket
- network spans: TCP bind, listen, connect, accept, send, recv
- provider labels from the rule-based provider classifier
- resource metrics for the captured process tree
- one diagnostic node for the expected attach-time bootstrap gap

The source storage for this run also contained stdio payload segments. The
current JSON graph export does not embed payload bytes; use `actrailviewer
payloads` and `actrailviewer payload` against regenerated storage when payload
bytes are needed.

## Regeneration

Build the tools:

```bash
cargo build --release -p ebpf_probe -p view
```

Ensure the `/tmp/actrail-example-trace-path-retry.*` paths configured in
`generation.conf` do not already exist, then run the live generator:

```bash
sysctl kernel.perf_event_paranoid=-1
./target/release/ebpf_probe verify-live --config examples/traces/extended-observation/generation.conf
sysctl kernel.perf_event_paranoid=2
```

Export the graph:

```bash
./target/release/actrailviewer \
  --config examples/traces/extended-observation/viewer.conf \
  export-json \
  --trace-id 1 \
  --output /tmp/actrail-example-trace-export.json
```

The generator should finish with `live verification passed`. If an open path is
only recoverable through userspace retry, the event metadata includes
`path_retry_source=process_vm_readv`.

`export-json` fails if the output file already exists. The checked-in
`trace.json` is the fixture copy; use a fresh output path for local validation.
Only point `--output` at `examples/traces/extended-observation/trace.json` when
you intentionally want to refresh the fixture and have removed or moved the old
file first.

This fixture was generated on WSL. The trace is completed but marked
`Degraded` because attaching to an already-running process records a bootstrap
gap diagnostic.
