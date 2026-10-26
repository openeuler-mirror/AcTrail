# Real-agent file state verification

```sh
python3 -m scripts.bench.payload.file_state.run \
  --agent-bin /home/yzh/.cargo/bin/xiaoo \
  --out local/bench/payload/file-state
```

The runner refreshes daemon defaults, applies the selected collection patch, and
runs xiaoo against HTTPS. Its actual shell tool executes a compiled C workload.
The workload checks FD duplication, descriptor flags, fork and exec inheritance,
directory-relative openat/openat2, private and shared mmap, FD reuse, rename, unlink and failed
syscalls. It verifies final file contents and saves its FD numbers as evidence.
Viewer readback checks file attribution and ordering using observed process
identities. Successful FD context events may be consumed by semantic projection;
the verifier checks their effects through subsequent file operations.

This is functional verification with two requests and one tool execution. CPU
comparison uses the separate ablation runner with its full workload.

`--config` selects a patch over refreshed defaults. `--mmap-only` verifies the
file attribution and output boundary of a configuration requesting only mmap
file observations. The full verifier also checks that an exec-closed directory
FD produces an unresolved relative path on EBADF.

`--context-probe-event` records
an existing perf uprobe on the current daemon's file decoder. The probe must expose
`aux`, `command` and `phase` from the actual uploaded header. The runner checks
that fcntl uploads contain only duplication/descriptor-flag commands, and that
fcntl, close and mmap upload only completed records. Probe registration must match the current binary and ABI;
remove the probe before CPU measurement.

`--hold-file-trace --host-ebpf disabled --no-file-observations` keeps a file
trace active while xiaoo requests a subset of its capabilities. Use a patch with
both direct TLS discovery switches disabled and `--context-probe-event GROUP:*`.
Register `context` (with `aux`), `tracker_seed`, `tracker_exec`, `tracker_inherit`
and `tracker_record` against the current daemon symbols. The verifier requires
baseline tracker activity, uploaded target context and zero target tracker calls.
Perf acknowledges readiness before either workload begins. This case verifies
real HTTPS/tool execution and absence of file state work; TLS plaintext coverage
is outside this configuration's verification scope.

`--bulk-read-retention full` or `--bulk-read-retention errors_only` runs the real
bulk reader. Select a matching patch with `file_observation.bulk_read.enabled=true`,
`mode="path_set"`, the requested `raw_event_retention`, and explicit
`file_observation.collection.read` demands. The helper reads A/B/C and produces
ENOENT around those reads. The verifier checks typed per-file deltas, raw summary
retention and the union of actual batch path sets. Failed opens remain separate
from read errors. Full retention verifies one operation and one byte for each
fixture file; errors-only verifies successful summaries are absent from raw
storage while the bulk actions retain the paths and I/O contributions.
