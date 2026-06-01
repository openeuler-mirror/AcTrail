# ARM64 BoringSSL Probe Point Methodology

This note describes a repeatable way to find AcTrail TLS plaintext read/write
uprobe points in an ARM64 program that uses static or hidden BoringSSL.

## Goal

Find runtime addresses that have `SSL_read` and `SSL_write` semantics:

```text
x0 = TLS/runtime object
x1 = plaintext buffer
x2 = plaintext length
```

The ELF may not expose `SSL_read` or `SSL_write` symbols. In that case, the
useful probe point may be a hidden BoringSSL function, an adapter wrapper, or an
internal callsite where arguments have already been normalized into the shape
above.

## Workflow

1. Identify the actual ELF executable, not just a launcher script.
2. Record architecture, GNU build ID, and binary hash.
3. Check `readelf -Ws` for `SSL_read`, `SSL_write`, or `SSL_write_ex`.
4. If symbols exist, prefer the symbol path and validate with a short E2E run.
5. If symbols are hidden on ARM64, first scan for the unified
   read/write entry signatures documented in
   `arm64-boringssl-ssl-write-entry-pattern.md`.
6. Require `SSL_read`, `SSL_read_internal`, and `SSL_write` signatures to each
   match exactly once.
7. Emit a build-id-bound symbol map only when both related read entries match
   the documented deltas from `SSL_write`, then validate with a real HTTPS
   workload.
8. If no unique signature relation exists, run a real HTTPS workload and collect
   syscall stacks around `sendto`, `sendmsg`, `write`, and `writev`.
9. Use the TLS record `sendto` stacks to find stable return addresses in the
   target executable.
10. Map those return addresses back to `.eh_frame` FDE ranges and disassemble
   the surrounding functions.
11. Look for the transition from plaintext buffer handling into encrypted
   network writes.
12. Probe candidate function entries or internal instruction addresses with
   temporary uprobes and capture `x0`, `x1`, and `x2`.
13. Accept a candidate only when runtime register shape and AcTrail E2E output
    both prove plaintext capture.

## Static Clues

Useful ARM64 clues include:

- `x1` preserved or forwarded as a buffer pointer.
- `w2` or `x2` preserved, bounded, truncated, or forwarded as a length.
- Calls into lower-level read/write/BIO wrappers after argument normalization.
- Return handling consistent with an integer byte count or negative error.
- A stable path from plaintext handling to TLS record `sendto` stacks.

Static clues are only candidate generators. They are not enough to declare the
address correct.

## Runtime Checks

Attach temporary uprobes and capture registers. A good candidate should:

- Hit during real HTTPS traffic.
- Hit in the expected process or tracked process tree.
- Show `x1` as a plausible user-space buffer pointer.
- Show `x2` as plausible plaintext operation lengths.
- Produce varied request-sized lengths, not only flags such as `0`, `1`, or
  constant scheduler values.
- Produce AcTrail payload rows from the intended BoringSSL TLS probe path.
- Produce a complete semantic `llm.request` for LLM traffic.

Reject candidates that only see ciphertext buffers, TLS record write sizes after
encryption, or unrelated state transitions.

## Pattern Extraction

After runtime validation:

1. Prefer the documented ARM64 read/write signature relation when it matches:
   unique `SSL_read`, unique `SSL_read_internal`, unique `SSL_write`, and the
   expected read/write deltas.
2. For a newly discovered candidate, extract bytes from the verified ARM64
   address and record the selected pattern length.
3. Search the entire target ELF for the byte sequence.
4. Require the chosen relation to hold against the unique `SSL_write` anchor.
5. Bind any produced symbol map to `arch` and GNU `build_id`.
6. Keep logical map symbols as `SSL_read` and `SSL_write` only if the probe
   points have the validated argument semantics.

Example symbol-map shape:

```text
resolver = bun-static-boringssl
library = boringssl
arch = aarch64
build_id = <target-build-id>
symbol = SSL_read|0x<verified-read-wrapper-virtual-address>
symbol = SSL_write|0x<verified-write-virtual-address>
```

## Portability Rule

Do not treat a verified address as portable. The portable unit is the ARM64
entry byte patterns plus the verified read/write relation from a unique
`SSL_write` anchor. Longer build-specific patterns can be recorded as forensic
evidence, but they should not become the default detector unless they prove
more portable across multiple ARM64 BoringSSL builds.

The reliable promotion path is:

```text
syscall stack evidence
-> disassembly and argument-flow analysis
-> temporary uprobe register validation
-> unique byte-pattern extraction
-> build-id-bound symbol map
-> AcTrail E2E semantic validation
```
