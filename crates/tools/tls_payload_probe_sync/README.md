# tls-payload-probe-sync

`tls-payload-probe-sync` launches a target process with a native preloaded runtime that installs Linux x86_64/aarch64 inline hooks at TLS plaintext probe points resolved by `tls_probe_point_finder`.

## Defaults

The CLI starts with these defaults:

- `--arch auto`
- `--provider auto`
- `--source auto`
- `--match-limit 8`
- `--max-payload-bytes 262144`
- `--redaction redact`
- `--events target,payload,decision`

## MVP Boundaries

The first sync runtime supports equal-byte payload rewrite only:

- outbound `SSL_write`, `SSL_write_ex`, and `rustls_buffer_plaintext`
- inbound `SSL_read`, `SSL_read_ex`, and `rustls_take_received_plaintext`
- Linux x86_64 and aarch64 native inline hooks
- original TLS connection semantics

It does not support compressed payload rewrite, variable-length rewrite, proxy replacement, or readiness emulation.

The aarch64 hook backend relocates the first 16 bytes of the target function into a trampoline. It rewrites `adr`/`adrp` and `bl` in that stolen range, and rejects other PC-relative control flow, PC-relative literal loads, or register branches there.

Rustls support uses the `Payload<'_>` and `OutboundChunks<'_>` layouts validated by the finder/BPF path. Replacement only writes to backing ranges that are still mapped writable.
