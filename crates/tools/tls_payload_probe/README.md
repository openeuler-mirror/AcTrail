# tls_payload_probe

`tls-payload-probe` launches one target command, resolves TLS payload probe points with `tls_probe_point_finder fast`, attaches pid-specific eBPF uprobes with libbpf, copies plaintext payload bytes inside BPF with `bpf_probe_read_user`, and streams binary payload events to the CLI. The CLI reports raw payload-event metadata, then assembles HTTP/1.x plaintext into body fragments and complete messages. Streaming `text/event-stream` bodies are reassembled into SSE frames, and recognized LLM text events are reported as a separate text layer.

It does not use tracefs `uprobe_events`, ftrace `trace_pipe`, or user-space `process_vm_readv` payload reads.

## CLI

```bash
target/debug/tls-payload-probe probe -- <agent-program> [args...]
```

Example:

```bash
target/debug/tls-payload-probe probe --provider rustls -- \
  xiaoo run -p "请直接回答：你好"
```

Default builds use the ring-buffer transport. To force perfbuffer and avoid ringbuf maps/helpers in this tool's BPF object:

```bash
cargo build -p tls_payload_probe --features perf-buffer
```

The first argument after `--` is the program inspected by finder fast mode. The target is spawned paused, BPF uprobes are attached to that pid, and then the target is resumed.

## Defaults

These constants live in `src/capture/config.rs` and are surfaced by CLI flags:

- `--max-capture-bytes`: `65535`
- `--ring-buffer-bytes`: `4194304`
- `--pending-ops`: `4096`
- `--match-limit`: `8`
- `--rustls-chunks`: `8`
- `--poll-ms`: `100`
- `--drain-ms`: `2000`
- `--assemble-buffer-bytes`: `4194304`
- `--decode-input-bytes`: `1048576`
- `--decode-output-bytes`: `4194304`
- `--decode-reader-buffer-bytes`: `4096`
- `--redaction`: `redact`
- `--events`: all event groups
- `--ring-stats`: disabled

`--library` and `--library-search-dir` are passed through to finder fast mode for OpenSSL shared-library resolution. Probe does not run the full finder report path.

`--redaction none` prints assembled HTTP text without masking headers or bearer tokens. `--events llm` prints only LLM projection events. Multiple event groups can be selected with comma-separated values, for example `--events http,sse,llm`. Supported groups are `payload`, `http`, `sse`, `llm`, and `target`. Event filtering only controls printing; upstream payload capture, HTTP assembly, SSE framing, and LLM projection still run so downstream groups remain available.

Raw `payload_event` lines do not print captured payload bytes. Incremental textual HTTP bodies are reported as `http_body_fragment` entries as soon as parseable body bytes arrive. Complete HTTP/1.x records are printed as `http_message` entries; if a body was already reported through fragments, the final message prints a streamed summary instead of repeating the body.

Chunked bodies are dechunked before display. `gzip`, `deflate`, `zstd`, and `br` bodies are decoded only when the compressed input is within `--decode-input-bytes` and decoded output stays within `--decode-output-bytes`; otherwise the message reports the explicit decode state instead of printing compressed bytes. If the target exits before an HTTP body is complete, the CLI prints a `partial` body state instead of treating capture as a probe failure.

For `text/event-stream`, HTTP body fragments print only a summary. The SSE assembler queues each fragment by pid, stream, and direction, reports each complete frame as `sse_frame`, emits inbound streaming text chunks as `llm_delta`, and emits inbound accumulated response text as `llm_message`. Complete frame boundaries are reported immediately; any remaining parseable partial frame is reported during shutdown as best effort.

LLM projection separates outbound request projection from inbound response projection. Outbound HTTP request bodies are projected as `llm_request` when the request matches Anthropic Messages, OpenAI Chat Completions, or OpenAI Responses schemas. Inbound OpenAI Responses streams match `response.output_text.delta` / `response.output_text.done`. Inbound Chat Completions streams match `choices[].delta.content`, `choices[].message.content`, `[DONE]`, or non-null `choices[].finish_reason`. Inbound Anthropic/Claude Messages responses match non-stream JSON objects with `type=message` and `content[].type=text`, and Anthropic Messages SSE streams match `content_block_delta`, `content_block_stop`, `message_delta`, and `message_stop`.

For streamed non-SSE HTTP bodies, projection caches body fragments by pid, stream, and direction, then uses the same `--decode-input-bytes`, `--decode-output-bytes`, and `--decode-reader-buffer-bytes` limits as HTTP reporting before parsing JSON. Projection parsers live under `src/llm_projection/inbound/` and `src/llm_projection/outbound/`; `capture/assembly/sse.rs` only assembles SSE frames and exposes frame data to the projection layer.

## BPF ABI

The BPF payload event header is 72 bytes followed by a size-classed payload region. Ring-buffer records reserve `72 + class_size` bytes, where `class_size` is the smallest bucket that fits `captured_size`: `512`, `2048`, `4096`, `8192`, or `65535`. Perfbuffer builds use a per-cpu scratch event and submit `72 + captured_size` bytes with `bpf_perf_event_output`; `--ring-buffer-bytes` is converted to perf pages. The single-event payload ABI maximum is `65535` bytes. `--ring-buffer-bytes` must be at least `--max-capture-bytes + 72`. OpenSSL/BoringSSL operations larger than one event are split into at most `8` ordered segments and reassembled before HTTP parsing. If the operation exceeds the configured segment budget, the last emitted segment carries the `truncated` flag and the HTTP assembler fails fast; the tool does not fall back to user-space reads. Rustls capture still emits one payload event per rustls payload/chunk path.

When `--ring-stats` is enabled, the CLI prints emitted record accounting and BPF loss counters after `target_exit`. `actual_bytes` uses `72 + captured_size`; `reserved_bytes` uses `72 + class_size`. Reserve failures are counted in BPF because those payloads never reach userspace. User-memory read failures are also counted before the reserved record is discarded. Perfbuffer builds also report BPF output failures and perf lost callback counts.

The BPF object is compiled for the host target architecture. x86_64 uses System V registers (`di`, `si`, `dx`, `cx`, `ax`); aarch64 uses `x0..x3` and `x0` return. `SSL_read` and `SSL_write` normalize their third argument as a positive `int`; `SSL_read` also normalizes its return value as a positive `int`. `SSL_read_ex` and `SSL_write_ex` treat the length argument as `size_t`.

Rustls payload capture uses the `Payload` word layout observed in the matched rustls functions:

- word 0 is either the inline payload tag or the chunk-array pointer
- word 1 is either the inline pointer or chunk count
- word 2 is either the inline length or range start
- word 3 is the range end for chunked outbound payloads
- the borrowed inbound payload tag is `0x8000000000000000`
- chunk entries are pointer/length pairs of two machine words
- BPF inspects at most `8` outbound chunks; `--rustls-chunks` can lower that value

OpenSSL `SSL_read_ex`/`SSL_write_ex` success is represented by return value `1`.

## Runtime Boundary

Reusable capture code is under `src/capture/`. CLI parsing and formatting are under `src/cli/`. The eBPF loader is isolated in `src/capture/ebpf.rs`, so a future daemon can reuse the capture runtime without taking CLI argument parsing or reporter formatting.

## Cleanup

Links are owned by the runtime and detached when the session drops. The target is killed if BPF load or attach fails after the process has been spawned paused.
