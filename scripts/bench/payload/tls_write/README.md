# OpenSSL write completion audit

`client.c` connects to a real local TLS server using dynamically linked OpenSSL.
It records every `SSL_write` request, return value and immediate `SSL_get_error`.
The short case enables partial writes and submits 128 KiB. The failure case uses
a nonblocking socket while the server pauses reads; after an observed WANT_WRITE,
a separate control connection asks the server to reset the TLS connection.
Both WANT_WRITE and a subsequent terminal error must occur for that case to pass.
The limited case disables partial writes and completes one 128 KiB write. Its
retained prefix must have PolicyLimited on every segment and successful operation
completion; summed segment original sizes must equal the actual return length.

Use an isolated daemon generated from fresh defaults with the P profile and
`configs/tls-bpf-copy.toml`, plus L4 payload storage enabled and TLS diagnostic
logging enabled at debug level. Keep the TLS operation limit at 65535 bytes;
this makes the requested write exceed the capture limit while successful partial
writes fit inside it. The driver starts each C client through actrailctl and
requires actual direct-capture observations, so linkage alone cannot pass admission.

```sh
python -m scripts.bench.payload.tls_write.run \
  --out local/bench/payload/bpf-copy/openssl-write \
  --ctl target/release/actrailctl \
  --config /absolute/isolated/actraild.conf \
  --database /absolute/isolated/data/actrail.sqlite \
  --daemon-log /absolute/isolated/log/actraild.log
```

The driver saves the effective configuration, dynamic linkage, client results,
daemon join logs and `audit.json`. It checks the recorded trace root PID, matches
sequential client calls to direct operation IDs, and queries persisted TLS segments.
Positive short returns must have exactly their returned byte count with complete,
successful operation metadata. Nonpositive returns must have explicit failed join
evidence and no stored segments. Missing calls or finalization fail the audit.

This is a network capture fixture. Run the real-agent acceptance separately for
LLM behavior. The audit provides no CPU comparison.
