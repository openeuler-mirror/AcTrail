# TLS Probe Point Finder

This helper detects executable TLS plaintext uprobe candidates from documented
BoringSSL byte signatures. `main.py` dispatches by ELF architecture; the actual
detectors live under `arm/` and `x86/`.

Command/path resolution is shared in `common/entry.py`, so ARM and x86 use the
same launcher-to-ELF behavior.

## Detect First

For an arbitrary executable, the user does not need to know an address. Run
`detect`; it scans the binary and prints candidate addresses and a build-id
bound symbol map:

```bash
python3 local/tls-probe-point/tls-probe-point-finder/main.py detect \
  /path/to/program
```

`--arch auto` reads the ELF machine type. `--arch aarch64` or `--arch x86_64`
can be used to fail fast if the binary is not the expected architecture.
`--match-limit` defaults to `8`.

If the binary argument is a command name such as `opencode`, the finder first
resolves it with `which`/`PATH`. If the resolved entry is a launcher rather than
an ELF, it only follows explicit, auditable cases. A sibling hidden ELF named
`.<entry-name>` is preferred, because package launchers commonly delegate to
that concrete runtime. If no sibling ELF exists, Node/Bun shebang runtimes are
used.

If the executable exports supported TLS symbols, the finder prints those first
and does not force byte-pattern detection. Hidden/static BoringSSL binaries use
the architecture-specific pattern detectors below.

## ARM64 Method

The ARM64 detector uses a related-entry method. It requires unique BoringSSL
`SSL_read`, `SSL_read_internal`, and `SSL_write` entry signatures, then verifies
the related read entries sit at fixed distances from `SSL_write`:

```text
SSL_read          = SSL_write - 0x3c0
SSL_read_internal = SSL_write - 0x2c0
```

Both derived locations must match the documented ARM64 read-entry byte
signatures. When the full relation is satisfied, the output contains:

```text
symbol = SSL_read|0x...
symbol = SSL_write|0x...
```

`SSL_read_internal` is printed under `detected offsets` as corroborating
evidence, but the candidate symbol map only includes the stable public
`SSL_read` and `SSL_write` entries. The signatures, deltas, and validation
samples are documented in
`../arm64-boringssl-ssl-write-entry-pattern.md`.

The production Rust resolver `payload_tls_resolver = boringssl-static` ports
this related-entry method for x86_64 and aarch64. It emits the same logical
`SSL_read` and `SSL_write` probe points without splitting the TLS collector by
CPU architecture.

## x86_64 Method

The x86_64 detector follows the existing AcTrail static-BoringSSL pattern
method: find `SSL_read`, validate the related `SSL_do_handshake` location, then
derive or search the nearby `SSL_write` location. It prints every related entry
point it can compute:

```text
symbol = SSL_do_handshake|0x...
symbol = SSL_read|0x...
symbol = SSL_write|0x...
```

The runtime may only attach the subset it supports, but the finder does not hide
the extra offsets it discovered.

## Runtime Verification

Use an address printed by `detect`:

```bash
python3 local/tls-probe-point/tls-probe-point-finder/main.py trace \
  /path/to/program \
  --address <address-from-detect> \
  --tracefs /sys/kernel/tracing \
  --group actrail_tls_probe \
  --target-timeout-seconds 20 \
  --sample-limit 20 \
  -- /path/to/workload
```

ARM64 traces fetch `x0/x1/x2`. x86_64 traces fetch `rdi/rsi/rdx`.

## Extracting New Signatures

`pattern` is not the first command for users. It is only for maintainers after
an address has already been obtained by `detect` or separate runtime analysis:

```bash
python3 local/tls-probe-point/tls-probe-point-finder/main.py pattern \
  /path/to/program \
  --address <known-or-verified-address> \
  --length 0x20
```

Only promote a new built-in signature after the extracted bytes are unique in
the target binary and runtime tracing confirms plaintext-length register
semantics.
