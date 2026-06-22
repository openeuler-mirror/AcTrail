# TLS Probe Point Finder

Detection has moved to the Rust workspace tool:

```bash
cargo build -p tls_probe_point_finder
target/debug/tls-probe-point-finder detect /path/to/program
```

The Rust implementation lives under `crates/tools/tls_probe_point_finder/` and supports:

- OpenSSL executable and shared-library symbol detection.
- OpenSSL shared-library detection follows the target's `DT_NEEDED` dependency graph and reports direct/transitive `libssl.so*` hits; it does not enumerate system `libssl` candidates unless the target graph references them.
- Static BoringSSL x86_64/aarch64 byte-pattern detection.
- Rustls debug-symbol detection through demangled `PlaintextSink::write/write_vectored` symbols. Runtime symbol-map keys remain `rustls_plaintext_*`, but they are not treated as exported ELF aliases.
- The maintenance `pattern` command for verified addresses.

The Python `detect` and `pattern` entrypoints are retired so new probe-point detection does not keep two implementations alive. The old Python trace helper remains here only for manual tracefs experiments until it is replaced or removed explicitly.

Human-readable reports are formatted only by the Rust reporter. It owns the two-space nesting indentation, unordered list markers for repeated entries, and ANSI highlighting: symbol names are cyan, unresolved `not_found` states are yellow, resolved addresses are green, candidate sources are blue, and candidate providers are magenta.

```bash
target/debug/tls-probe-point-finder pattern \
  /path/to/program \
  --address <known-or-verified-address> \
  --length 0x20
```
