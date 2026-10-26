/// Default TLS sync unknown-stream capture window.
///
/// This is intentionally larger than small HTTP payloads because LLM requests can
/// carry prompt context, tool schemas, and file excerpts before the stream is
/// positively classified.
pub const DEFAULT_TLS_SYNC_FLOW_UNKNOWN_STREAM_BYTES: u64 = 1024 * 1024;

/// Maximum complete TLS sync IPC frame, including its header (32 MiB).
/// Configured independently from the captured operation limit to bound plan replies too.
pub const DEFAULT_TLS_SYNC_MAX_FRAME_BYTES: u32 = 32 * 1024 * 1024;

/// Fixed binary TLS IPC header size, included in the frame limit.
pub const TLS_SYNC_FRAME_HEADER_BYTES: usize = 8;
