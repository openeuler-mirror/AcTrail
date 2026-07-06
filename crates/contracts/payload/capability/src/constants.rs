/// Default TLS sync unknown-stream capture window.
///
/// This is intentionally larger than small HTTP payloads because LLM requests can
/// carry prompt context, tool schemas, and file excerpts before the stream is
/// positively classified.
pub const DEFAULT_TLS_SYNC_FLOW_UNKNOWN_STREAM_BYTES: u64 = 1024 * 1024;
