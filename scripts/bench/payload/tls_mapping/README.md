# TLS mapping boundaries

Run real xiaoo bash-tool calls against an isolated BpfCopy daemon:

```sh
python -m scripts.bench.payload.tls_mapping.run \
  --out local/bench/payload/tls-mapping-acceptance
```

The runner compiles `client.c` with `-ldl`, checks that it has no startup dependency
on libssl/libcrypto, and uses fresh default configuration with the P overlay and
file observation disabled. It reuses the payload benchmark's local MaaS TLS server,
real xiaoo workload, profile checks and isolated daemon lifecycle.

Three successive bash calls exercise independent copied library inodes:

1. Map libssl without EXEC, confirm the actual process maps, then successfully
   add EXEC with mprotect. The coordinator observes SharedLibrary attachment ready
   for that inode before allowing dlopen and a real HTTPS request.
2. Atomically replace the same library path with a new inode and repeat the
   operation and HTTPS request. This verifies different file versions even when
   their library contents are identical.
3. Unlink the fresh library after the non-executable mapping and before mprotect.
   Confirm deleted executable mappings and attachment, then load the held FD via
   `/proc/self/fd/N` and complete HTTPS.

The C client's TLS peer is the fixture on 127.0.0.1; certificate verification is
disabled for this local peer. The plaintext coordination connection only controls
test phases. It adds no product hook. Every successful response must reach DONE,
all seven outer/client LLM exchanges must pass P retention checks, and each client
must have its own valid command-to-LLM relationship. File events must remain absent.

Outputs include effective configuration, dynamic linkage, HTTP responses, daemon
logs, viewer graphs and `acceptance.json` with process generations, mapping snapshots,
attachment evidence and the recorded permission to proceed to dlopen.

The full-file executable mmap exists to exercise mprotect discovery; actual TLS
code runs through the subsequent dlopen mapping. Explicit test coordination ensures
discovery before HTTPS, so these results do not measure first-call coverage or CPU.
Unlink-before-mprotect verifies an event carrying the post-unlink identity. It does
not establish behavior when ctime changes after pinning. Pin followed by process
exit before attachment remains TODO: existing logs cannot deterministically enforce
that intermediate timing without a new product test hook.
