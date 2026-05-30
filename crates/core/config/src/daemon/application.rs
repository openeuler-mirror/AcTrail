//! Application-layer semantic analyzer configuration.

use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SseDataPolicy {
    Disabled,
    Preview,
}

impl FromStr for SseDataPolicy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "disabled" => Ok(Self::Disabled),
            "preview" => Ok(Self::Preview),
            other => Err(format!("unsupported SSE data policy {other}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationProtocolConfig {
    pub enabled: bool,
    pub http1_enabled: bool,
    pub http2_enabled: bool,
    pub capture_host: bool,
    pub sse_enabled: bool,
    pub sse_data_policy: SseDataPolicy,
    pub sse_max_buffer_bytes: u64,
    pub sse_max_data_bytes: u64,
    pub http2_max_frame_bytes: u64,
    pub http2_max_connection_buffer_bytes: u64,
    pub http2_emit_data_preview: bool,
    pub http2_max_data_preview_bytes: u64,
}
