# x86_64 BoringSSL Pattern Detector

This note documents the x86_64 static-BoringSSL detector used by `tls-probe-point-finder/x86/`. It mirrors the existing AcTrail agent example logic and reports every related entry point it can derive, not only `SSL_write`.

## Signatures

`SSL_do_handshake`:

```text
55 48 89 e5 41 57 41 56 41 55 41 54 53 48 83 ec 28
49 89 fc 48 8b 47 30
```

`SSL_read`:

```text
55 48 89 e5 41 57 41 56 53 50 48 83 bf 98 00 00 00 00 74
```

`SSL_write`:

```text
55 48 89 e5 41 57 41 56 41 55 41 54 53 48 83 ec 18
41 89 d7 49 89 f6 48 89 fb
```

## Layout Relations

The detector uses these documented x86_64 layout relations:

```text
SSL_read - SSL_do_handshake = 0x6f0
SSL_write - SSL_read = 0xca0
SSL_write fallback search window after SSL_read = 0x10000
```

The primary anchor is a unique `SSL_read` match. The detector then validates the expected handshake location and expected write location. If the write delta does not match directly, it searches for a unique `SSL_write` signature in the documented window after `SSL_read`.

## Output

When successful, the x86_64 detector prints all derived entry points:

```text
symbol = SSL_do_handshake|0x...
symbol = SSL_read|0x...
symbol = SSL_write|0x...
```

The AcTrail runtime may attach only the supported subset. The finder output is intentionally broader so reviewers can inspect the complete relation that made the `SSL_write` offset credible.
