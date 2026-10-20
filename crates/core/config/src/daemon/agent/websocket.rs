//! WebSocket transport retention and provider conversation limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct L2WebSocketRetention {
    pub enabled: bool,
    pub message_summary: bool,
    pub max_active_connections: u32,
    pub max_frame_buffer_bytes: u64,
    pub max_message_bytes: u64,
    pub max_decoded_message_bytes: u64,
    pub max_evidence_ranges: u32,
    pub compact_threshold_bytes: u64,
}
impl Default for L2WebSocketRetention {
    fn default() -> Self {
        Self {
            enabled: true,
            message_summary: true,
            max_active_connections: 4096,
            max_frame_buffer_bytes: 8 * 1024 * 1024,
            max_message_bytes: 16 * 1024 * 1024,
            max_decoded_message_bytes: 32 * 1024 * 1024,
            max_evidence_ranges: 4096,
            compact_threshold_bytes: 64 * 1024,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LlmWebSocketConfig {
    pub max_lanes_per_connection: u32,
    pub max_pending_requests_per_connection: u32,
}
impl Default for LlmWebSocketConfig {
    fn default() -> Self {
        Self {
            max_lanes_per_connection: 32,
            max_pending_requests_per_connection: 32,
        }
    }
}
