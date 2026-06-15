# xiaoO Rustls LLM Capture

This guide captures HTTPS LLM requests from the Rust xiaoO binary without changing xiaoO code. The current path uses `actrailctl launch`, `tls-sync`, and the finder fast auto plan. It does not require a hand-written rustls symbol map.

```text
xiaoO rustls plaintext probe points
  -> actrailctl launch resolves the auto plan
  -> actrailctl launch prepares the tls-sync runtime and event socket
  -> the runtime reports plaintext bytes at the hook point
  -> actrailviewer shows payload rows, actions, JSON, and OTLP spans
```

## 1. Build Binaries

Build AcTrail first:

```bash
cargo build --release
```

Use the xiaoO binary that will run in production. The default path is the shell `PATH`:

```bash
XIAOO_BINARY="$(command -v xiaoo)"
test -x "$XIAOO_BINARY"
```

If you need to validate a specific checkout instead of the installed command, build that project and set `XIAOO_BINARY` explicitly:

```bash
XIAOO_BINARY=/path/to/xiaoo
test -x "$XIAOO_BINARY"
```

## 2. Check Rustls Auto Plan

Run the same fast resolver that `actrailctl launch` uses:

```bash
target/release/tls-probe-point-finder \
  fast \
  --provider rustls \
  --source auto \
  --match-limit 8 \
  "$XIAOO_BINARY"
```

The output must contain `provider = rustls` plus both payload hook points:

```text
rustls_buffer_plaintext
rustls_take_received_plaintext
```

If the finder cannot resolve those hook points for this xiaoO binary, stop there. Do not switch to a different binary to satisfy the check.

## 3. Resolve Config

Write a concrete config from the template:

```bash
XIAOO_CONFIG=/tmp/actrail-xiaoo-rustls.conf
cp docs/llm-capture/xiaoo-rustls/operator.conf "$XIAOO_CONFIG"
```

The template uses auto TLS fields because `tls-sync` launch resolves the actual executable rustls plan at startup:

```text
payload_tls_capture_backend = tls-sync
payload_tls_source = auto
payload_tls_resolver = auto
payload_tls_library = auto
payload_tls_binary_path = disabled
payload_tls_pattern_path = disabled
payload_tls_sync_runtime_library_path = auto
payload_tls_sync_match_limit = 8
```

## 4. Run Capture

Clean the configured runtime files, start the daemon, and check control-plane health:

```bash
./target/release/actrailctl clean --config "$XIAOO_CONFIG"
./target/release/actraild --config "$XIAOO_CONFIG" start
./target/release/actrailctl doctor --config "$XIAOO_CONFIG"
```

Use xiaoO's default config first. On this machine that config is `~/.config/xiaoo/config.toml`, with provider/model/API-key-env already configured by xiaoO. Do not pass `--api-key` on the command line; process argv is intentionally observable in AcTrail traces.

Run a real xiaoO LLM prompt through `actrailctl launch`:

```bash
./target/release/actrailctl \
  --config "$XIAOO_CONFIG" \
  launch \
  --name xiaoo-rustls-llm \
  -- \
  "$XIAOO_BINARY" run \
    --no-tools \
    --max-turns 1 \
    --prompt "请用一句话回答：xiaoO rustls 采集测试"
```

The API-key environment variable named by xiaoO's default config must be present in the environment. Keep the xiaoO process under `actrailctl launch`; do not use `track-add` for this TLS capture path.

## 5. Verify Trace

List traces and pick the trace id from the launch result:

```bash
./target/release/actrailctl list-traces --config "$XIAOO_CONFIG"
```

Check raw TLS payloads:

```bash
./target/release/actrailviewer payloads \
  --config "$XIAOO_CONFIG" \
  --trace-id <TRACE_ID> \
  --head 20
```

Expected payload rows:

```text
SOURCE=TlsUserSpace
DIRECTION=Outbound
LIBRARY=rustls
SYMBOL=rustls_buffer_plaintext
OPERATION_STATE=success
TRUNCATION=Complete
```

Check semantic actions:

```bash
./target/release/actrailviewer actions \
  --config "$XIAOO_CONFIG" \
  --trace-id <TRACE_ID>
```

Expected action row:

```text
KIND=llm.request
STATUS=success
COMPLETENESS=complete
```

Export JSON with retained payload bytes/text:

```bash
./target/release/actrailviewer export-json \
  --config "$XIAOO_CONFIG" \
  --trace-id <TRACE_ID> \
  --output /tmp/actrail-xiaoo-rustls.json
```

The exported graph should contain `Payload` nodes with non-empty `bytes_base64` and `text`.

## Failure Modes

AcTrail fails fast if `actrailctl launch` cannot resolve a complete rustls auto plan, the sync runtime library is missing, the event socket cannot be prepared, or the trace is started with `track-add` instead of `launch`. It does not fall back to encrypted socket bytes.
