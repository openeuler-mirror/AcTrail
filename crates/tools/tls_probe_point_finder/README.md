# TLS Probe Point Finder

Rust workspace tool for finding TLS plaintext uprobe points.

Build:

```bash
cargo build -p tls_probe_point_finder
```

Run:

```bash
target/debug/tls-probe-point-finder -h
target/debug/tls-probe-point-finder detect -h
target/debug/tls-probe-point-finder fast -h
target/debug/tls-probe-point-finder detect codex
target/debug/tls-probe-point-finder fast /path/to/agent
target/debug/tls-probe-point-finder detect --provider=boringssl --source=executable opencode
target/debug/tls-probe-point-finder pattern codex --address 0x1a66950 --length 0x20
```

## Detect

`detect` resolves a command or path to a concrete ELF file, reads ELF metadata directly, and reports provider candidates:

- `openssl` executable symbols: requires `SSL_read`, `SSL_write`, `SSL_read_ex`, and `SSL_write_ex`.
- `openssl` shared library symbols: reports user-specified, direct `DT_NEEDED`, and transitive `DT_NEEDED` `libssl.so*` candidates with separate confidence. Python executables are also checked by importing `_ssl` with `-S` and following that extension module's direct `DT_NEEDED` `libssl.so*` dependency; this handles native Python and uv virtualenv launchers that map OpenSSL only after `import _ssl`.
- `boringssl` executable symbols: available with `--provider boringssl`; auto mode does not treat shared `SSL_*` names alone as proof of BoringSSL.
- `boringssl` executable byte patterns: built-in x86_64/aarch64 related-entry detection for stripped static BoringSSL binaries.
- `rustls` executable symbols: uses the target ELF symbol table and local demangling for `rustls::common_state::CommonState::buffer_plaintext` and `rustls::common_state::CommonState::take_received_plaintext`, then emits the runtime `rustls_buffer_plaintext` and `rustls_take_received_plaintext` symbol-map keys.
- `rustls` executable byte patterns: x86_64/aarch64 entry-pattern detection for stripped rustls binaries. These patterns target plaintext inside rustls, not wrapper `PlaintextSink` or `tokio-rustls` call sites.
- `go` executable pclntab symbols: resolves `crypto/tls.(*Conn).Write`, `crypto/tls.(*Conn).Read`, and `runtime.memmove` from `.gopclntab`, including stripped Go binaries. The eBPF collector can use `payload_tls_resolver = go-pclntab` with `payload_tls_library = go` for Go standard-library HTTPS request and response direct-copy capture without Go return uprobes.

`detect` accepts `--provider auto|openssl|boringssl|rustls|go`, `--source auto|executable|shared-library`, and `--arch auto|aarch64|x86_64`. The documented default `--match-limit` is `8`; decimal and `0x` integer forms are accepted.

Detection and rendering are separate. The detection command builds an internal report structure, and `reporter.rs` is the only module that controls terminal formatting, list markers, and indentation. Human-readable output uses two spaces per nesting level.

## Fast

`fast` resolves a command or path to a concrete ELF file and returns the first complete payload-capture probe plan. It is intended for startup-sensitive tools that need attach points, not full human-readable evidence.

`fast` requires a complete payload closure before returning:

- rustls requires both `rustls_buffer_plaintext` and `rustls_take_received_plaintext`.
- OpenSSL requires `SSL_read`, `SSL_write`, `SSL_read_ex`, and `SSL_write_ex`.
- BoringSSL static pattern probing must resolve the provider's related read/write entry set; a single isolated byte-pattern hit is not enough.
- Go requires `crypto/tls.(*Conn).Write`, `crypto/tls.(*Conn).Read`, and `runtime.memmove` from `.gopclntab`. The generated plan marks `Conn.Read` as the read-side state point and `runtime.memmove` as the inbound copy point, so it does not require Go return uprobes.

The fast path tries cheaper lookups before slower scans:

- executable symbol-table matches,
- user-specified or direct `DT_NEEDED` OpenSSL shared libraries,
- executable `.gopclntab` matches for Go,
- executable static byte-pattern matches,
- transitive `DT_NEEDED` OpenSSL shared libraries.

It does not invoke the `tls-probe-point-finder` CLI recursively and does not run `nm`; future capture tools should import the crate and call the fast resolver directly.

## Pattern

`pattern` extracts bytes from a known virtual address and reports how many times the byte sequence appears in the ELF. It is for maintaining stripped binary signatures after an address has been verified elsewhere.

`pattern --address`, `--length`, and `--match-limit` accept decimal or `0x` integer values.

## Documented Constants

The ELF parser constants live under `src/elf/constants.rs` and cover ELF64 little-endian x86_64/aarch64 headers, program headers, section headers, symbol tables, notes, and dynamic entries.

OpenSSL shared-library probing resolves library names that are already present in the target ELF's direct or transitive `DT_NEEDED` dependency graph. It uses these documented dependency-resolution directories:

- `/lib`
- `/lib64`
- `/usr/lib`
- `/usr/lib64`
- `/lib/x86_64-linux-gnu`
- `/usr/lib/x86_64-linux-gnu`
- `/lib/aarch64-linux-gnu`
- `/usr/lib/aarch64-linux-gnu`

These paths do not create standalone system candidates. They are only searched after a `DT_NEEDED` entry names a dependency, or when the user passes `--library`.

Rustls stripped x86_64 probing uses these documented entry patterns:

- `rustls_buffer_plaintext`, `CommonState::buffer_plaintext`, 27 bytes: `55 41 57 41 56 41 55 41 54 53 48 83 ec 28 49 89 d6 48 89 f3 4c 8b a7 08 03 00 00`
- `rustls_take_received_plaintext`, `CommonState::take_received_plaintext`, 32 bytes: `41 57 41 56 41 54 53 50 49 89 ff c6 87 2e 03 00 00 20 4c 8b 26 4c 8b 76 08 4c 89 e0 48 f7 d8 48`

Rustls stripped aarch64 probing uses these documented entry patterns, collected from stripped `xiaoo`/`xiaoo-tui` aarch64 rustls 0.23.40 ThinLTO release binaries and verified with `pattern` as unique in both binaries:

- `rustls_buffer_plaintext`, `CommonState::buffer_plaintext`, 52 bytes: `ff 83 01 d1 fd 7b 02 a9 f8 5f 03 a9 f6 57 04 a9 f4 4f 05 a9 fd 83 00 91 17 84 41 f9 08 00 f0 d2 f4 03 02 aa f3 03 01 aa f5 03 00 aa 08 84 01 f9 ff 02 08 eb`
- `rustls_take_received_plaintext`, `CommonState::take_received_plaintext`, 64 bytes: `fd 7b bc a9 f7 0b 00 f9 f6 57 02 a9 f4 4f 03 a9 fd 03 00 91 37 50 40 a9 09 00 f0 d2 33 08 40 f9 f5 03 00 aa 08 04 80 52 08 b8 0c 39 ff 02 09 eb a1 00 00 54 73 01 f8 b6 e0 03 1f aa e1 03 13 aa`
