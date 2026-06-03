# xiaoO Rustls LLM Capture

This example captures HTTPS LLM requests from the Rust xiaoO binary without changing xiaoO code. The path is `actrailctl launch` + executable uprobes + seccomp user-read:

```text
xiaoO rustls PlaintextSink::write/write_vectored
  -> eBPF records pointer metadata and stops the thread
  -> actraild reads plaintext bytes from the stopped process
  -> actrailviewer shows payload rows, actions, JSON, and OTLP spans
```

## 1. Build Binaries

Build AcTrail first:

```bash
cargo build --release
```

Use the xiaoO binary that will run in production. The default path is the shell
`PATH`:

```bash
XIAOO_BINARY="$(command -v xiaoo)"
test -n "$XIAOO_BINARY"
```

If you need to validate a specific checkout instead of the installed command,
build that project and set `XIAOO_BINARY` explicitly:

```bash
XIAOO_BINARY=/path/to/xiaoo
test -x "$XIAOO_BINARY"
```

Use the same executable for symbol-map generation, `payload_tls_binary_path`, and `actrailctl launch`. If you rebuild xiaoO, regenerate the symbol map because the GNU build-id can change.

## 2. Generate Rustls Symbol Map

AcTrail validates the target executable build-id before attaching. Generate a map for the exact binary:

```bash
XIAOO_BINARY="${XIAOO_BINARY:-$(command -v xiaoo)}"
test -x "$XIAOO_BINARY"
XIAOO_MAP=/tmp/actrail-xiaoo-rustls.map
BUILD_ID="$(readelf -n "$XIAOO_BINARY" | awk '/Build ID:/{print $3; exit}')"
ARCH="$(uname -m)"

{
  printf 'resolver = rustls-symbol-map\n'
  printf 'library = rustls\n'
  printf 'arch = %s\n' "$ARCH"
  printf 'build_id = %s\n' "$BUILD_ID"
  nm -C "$XIAOO_BINARY" | awk '/PlaintextSink>::write$/ {print "symbol = rustls_plaintext_write|0x"$1}'
  nm -C "$XIAOO_BINARY" | awk '/PlaintextSink>::write_vectored$/ {print "symbol = rustls_plaintext_write_vectored|0x"$1}'
} > "$XIAOO_MAP"

cat "$XIAOO_MAP"
```

For stripped release binaries, first look for debuginfo instead of giving up.
The automated E2E checks these sources in order:

```text
1. main xiaoO binary symbol table
2. XIAOO_DEBUG_FILE, if set
3. /usr/lib/debug/.build-id/<build-id-prefix>/<build-id-rest>.debug
```

If you have a separate debuginfo file, set it before running the E2E:

```bash
export XIAOO_DEBUG_FILE=/path/to/xiaoo.debug
python3 tests/agent-trace/run_case.py xiaoo-rustls
```

Expected map shape:

```text
resolver = rustls-symbol-map
library = rustls
arch = x86_64
build_id = <target build id>
symbol = rustls_plaintext_write|0x...
symbol = rustls_plaintext_write_vectored|0x...
```

If either `symbol = ...` line is missing from both the executable and its
debuginfo, AcTrail has no plaintext rustls function address to attach for that
release binary. Stop there and provide matching debuginfo or a future
build-id-checked rustls pattern map for that exact release; do not switch to
another binary without regenerating the map.

## 3. Resolve Config

Write a concrete config from the template:

```bash
XIAOO_BINARY="${XIAOO_BINARY:-$(command -v xiaoo)}"
test -x "$XIAOO_BINARY"
XIAOO_MAP=/tmp/actrail-xiaoo-rustls.map
XIAOO_CONFIG=/tmp/actrail-xiaoo-rustls.conf

sed \
  -e "s#__XIAOO_BINARY__#$XIAOO_BINARY#g" \
  -e "s#__XIAOO_RUSTLS_SYMBOL_MAP__#$XIAOO_MAP#g" \
  docs/llm-capture/xiaoo-rustls/operator.conf > "$XIAOO_CONFIG"
```

The config uses:

```text
payload_tls_source = executable
payload_tls_resolver = rustls-symbol-map
payload_tls_library = rustls
payload_tls_capture_backend = seccomp-user-read
payload_tls_redaction_policy = authorization-header
payload_tls_sync_runtime_library_path = auto
payload_tls_sync_event_socket_path = /tmp/actrail-tls-sync.sock
payload_tls_sync_socket_mode_octal = 660
payload_tls_sync_match_limit = 8
application_protocol_http1_enabled = true
```

xiaoO's current `reqwest` path is configured for rustls and `http1_only()`, so the expected semantic output is HTTP/1.x request content derived from TLS plaintext.

## 4. Run Capture

Clean the configured runtime files, start the daemon, and check control-plane health:

```bash
./target/release/actrailctl clean --config "$XIAOO_CONFIG"
./target/release/actraild start --config "$XIAOO_CONFIG"
./target/release/actrailctl doctor --config "$XIAOO_CONFIG"
```

Use xiaoO's default config first. On this machine that config is `~/.config/xiaoo/config.toml`, with provider/model/API-key-env already configured by xiaoO. Do not pass `--api-key` on the command line; process argv is intentionally observable in AcTrail traces.

If xiaoO exits before making an LLM request with an error like `unknown field physical`, the default config contains an `operation_backend` block from a different xiaoO schema. Back it up and comment only that incompatible block before rerunning:

```bash
cp ~/.config/xiaoo/config.toml ~/.config/xiaoo/config.toml.actrail-backup
```

Then comment the `[operation_backend]` section through the end of the file, leaving the `[llm]` section and its `api_key_env` unchanged.

Run a real xiaoO LLM prompt through `actrailctl launch`. The provider/model come from xiaoO's default config:

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
SYMBOL=rustls_plaintext_write or rustls_plaintext_write_vectored
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

Export pretty OTLP JSON:

```bash
./target/release/actrailviewer export-otel \
  --config "$XIAOO_CONFIG" \
  --trace-id <TRACE_ID> \
  --output /tmp/actrail-xiaoo-rustls.otlp.json
```

Expected OTLP content includes a span with:

```text
actrail.action.kind = llm.request
actrail.action.status = success
actrail.action.completeness = complete
http.request.method = POST
http.request.body_text = <non-empty JSON request body>
llm.request.raw_payload_base64 = <non-empty base64>
```

## Failure Modes

AcTrail fails fast if the binary path does not exist, the symbol map does not exist, the map resolver/library/build-id/arch does not match, either rustls symbol is missing, or the trace is started with `track-add` instead of `launch`. It does not fall back to encrypted socket bytes.
