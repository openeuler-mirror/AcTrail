# Shared-library TLS discovery acceptance

Run the real xiaoo agent against local HTTPS MaaS. Its two bash tool calls execute
curl with a private copy of the system `libssl.so.3`, selected through the curl
process's `LD_LIBRARY_PATH`. The copied library preserves the actual OpenSSL
implementation while providing a new inode for discovery.

```sh
python3 -m scripts.bench.payload.tls_library.run --out local/bench/payload/bl
```

`--agent-bin`, `--curl`, `--libssl`, and `--bin-dir` specify executable and library
locations. Each profile initializes an isolated daemon from fresh defaults and
the repository P profile plus the explicit bpf-copy overlay. The second profile
removes `fs-access-basic` and `fs-mmap`. Both reuse one library inode across the
two curl tool calls. External daemons remain running.

The first curl request can report an asynchronous discovery coverage gap. The
second requires a complete request, response, call, and command association.
The outer agent must complete all three model requests and both tool calls.
Client responses must contain the SSE completion marker, and MaaS must record
exactly two successful HTTPS requests. The server uses 30 ms TPOT; there is no
delay before a client's first HTTPS call.

Artifacts include refreshed configuration, binary hashes, actual argv, copied
library identity, curl process maps, real response bytes, MaaS logs, database
action/event exports, and discovery diagnostics. Maps must demonstrate use of
the private library without the injected TLS runtime. File-disabled runs require
zero File events and file/fs semantic actions. Process attribution checks use
the stored process identity and host PID generation.

This fixture provides functional evidence. It does not measure CPU or establish
coverage of explicit dlopen/mprotect calls or process-exit races. Loader-created
mapping splits can be checked against the saved maps and discovery diagnostics.
