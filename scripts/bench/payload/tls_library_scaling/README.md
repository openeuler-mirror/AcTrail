# Real-agent shared-library quantity benchmark

```sh
python3 -m scripts.bench.payload.tls_library_scaling.run --out local/bench/payload/ls
python3 -m scripts.bench.payload.tls_library_scaling.summary --out local/bench/payload/ls
```

Use an existing release with `--bin-dir`, real xiaoo with `--agent-bin`, system
curl with `--curl`, and the actual OpenSSL library with `--libssl`. The runner
does not build Rust or native fixtures. It only creates scripts and copies the
library before measurement.

N=10/20/30 × same/fresh library × bare/tls-sync/bpf-copy × one warmup and three
measurements yields 72 runs: 18 warmups and 54 measurements. The 12 observed
groups each keep one isolated daemon running for all four runs; six bare groups
launch xiaoo directly. Both P profiles derive from refreshed defaults, set the
same 65535-byte limits, and differ only in TLS backend. Effective configuration
is compared after normalizing isolation paths.

Each run has a fixed two-turn outer xiaoo conversation and one real bash tool
invocation executing N curl processes sequentially. The outer tool returns
128 bytes. Curl performs a fresh HTTPS connection and receives a complete SSE
message from local MaaS. TPOT is 0 and no artificial pre-request delay is used.

Same means one newly copied library inode shared by all N curl processes within
that run. Fresh means N different inodes, one per curl. Every run, including
warmup, has new target files; files remain on disk to prevent inode reuse.
The original executable and common dependency caches may be warmed across the
group. Copies retain exactly the original system OpenSSL contents.

Task CPU is measured with wait4 through the outer process, including reaped
bash/curl descendants. Daemon CPU is sampled before launch, at exit, and after
trace drain. Library/script preparation, MaaS/controller CPU, validation and
daemon startup/shutdown are excluded. No perf or procfs maps scanner runs in
the measured window. Existing library functional fixtures establish the load
mechanism; this benchmark records explicit commands, library versions, real
HTTPS results and retained process/action identities.

Real business must produce N successful curl/SSE responses and the complete
outer conversation/tool result. Capture misses and incomplete observations are
reported per curl and retained in CPU statistics. Invalid cross-process links,
duplicate LLM actions, failed business or unclean traces fail validation. Reduced
CPU with unequal coverage is not presented as equivalent-capability savings.

Summary artifacts include absolute means/ranges, comparisons with bare and
tls-sync denominators, adjacent N growth, capture coverage and discovery counts.
Attachment-ready counts include reuse. Raw runs preserve warmup/measurement
results, all response files, scripts, library identities, daemon logs and SQLite
databases. External daemons are recorded and never stopped.
