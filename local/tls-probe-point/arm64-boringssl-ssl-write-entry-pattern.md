# ARM64 BoringSSL SSL_read/SSL_write Entry Patterns

This note records the unified ARM64 detector used to find static or hidden
BoringSSL plaintext uprobe points. The detector is not tied to a single
application. The application binaries below are validation samples used to test
the detector.

## Unified Signatures

The default ARM64 detector scans for this 32-byte `SSL_write` function-entry
signature:

```text
ff 03 01 d1 fd 7b 01 a9 f6 57 02 a9 f4 4f 03 a9
fd 43 00 91 08 18 40 f9 f5 03 02 2a f4 03 01 aa
```

The corresponding AArch64 instruction shape is:

```text
sub sp, sp, #0x40
stp x29, x30, [sp, #16]
stp x22, x21, [sp, #32]
stp x20, x19, [sp, #48]
add x29, sp, #0x10
ldr x8, [x0, #48]
mov w21, w2
mov x20, x1
```

This preserves the expected `SSL_write(ssl, buf, len)` argument shape:

```text
x0 = TLS/runtime object
x1 = plaintext buffer
x2 = plaintext length
```

The detector accepts the relation only when all three signatures match exactly
once in the target executable.

It then validates the related read entries at fixed distances before emitting a
complete read/write symbol map:

```text
SSL_read          = SSL_write - 0x3c0
SSL_read_internal = SSL_write - 0x2c0
```

The `SSL_read` wrapper must match this 24-byte function-entry signature:

```text
fd 7b bd a9 f5 0b 00 f9 f4 4f 02 a9 fd 03 00 91
08 4c 40 f9 a8 01 00 b4
```

The corresponding AArch64 instruction shape is:

```text
stp x29, x30, [sp, #-48]!
str x21, [sp, #16]
stp x20, x19, [sp, #32]
mov x29, sp
ldr x8, [x0, #152]
cbz x8, <read-path>
```

The internal read entry must match this 32-byte function-entry signature:

```text
ff 03 02 d1 fd 7b 04 a9 f8 5f 05 a9 f6 57 06 a9
f4 4f 07 a9 fd 03 01 91 08 18 40 f9 f3 03 00 aa
```

The corresponding AArch64 instruction shape is:

```text
sub sp, sp, #0x80
stp x29, x30, [sp, #64]
stp x24, x23, [sp, #80]
stp x22, x21, [sp, #96]
stp x20, x19, [sp, #112]
add x29, sp, #0x40
ldr x8, [x0, #48]
mov x19, x0
```

`SSL_read_internal` is emitted only as corroborating evidence in `detected
offsets`. The candidate symbol map uses the public `SSL_read` wrapper plus
`SSL_write`.

## Validation Samples

The same 32-byte signature was validated on these ARM64 static-BoringSSL
executables:

| Sample | Build ID | Match | Runtime evidence |
| --- | --- | --- | --- |
| Claude native executable | `c0cb5d4146c5c3d8a27e5c6e6f47c4043db46563` | `SSL_read=0x4131c00`, `SSL_read_internal=0x4131d00`, `SSL_write=0x4131fc0` | Real LLM request captured `SSL_read` wrapper hits with `x2 <= 0x10000`; `SSL_write` positive control hit with outbound plaintext lengths. |
| OpenCode executable | `b5b74501ad8ae6855b83b56e7ec1e6ef5eae266f` | `SSL_read=0x3976880`, `SSL_read_internal=0x3976980`, `SSL_write=0x3976c40` | Real LLM request captured `SSL_read` wrapper hits with `x2 <= 0x10000`; `SSL_write` positive control hit with outbound plaintext lengths. |

The samples are evidence for the shared read/write relation. They are not
separate application-specific detection methods.

## Non-Default Longer Patterns

The first 48 bytes differed between the validation samples because the next call
target and surrounding layout changed by build. Those longer sequences are
useful for forensic comparison, but they are not part of the default unified
detector.

## Symbol Map Shape

For a unique match, emit:

```text
resolver = bun-static-boringssl
library = boringssl
arch = aarch64
build_id = <target-build-id>
symbol = SSL_read|0x<matched-read-wrapper-virtual-address>
symbol = SSL_write|0x<matched-write-virtual-address>
```
