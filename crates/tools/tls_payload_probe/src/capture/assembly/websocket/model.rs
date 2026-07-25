//! Assembled WebSocket message model.

use crate::capture::CaptureDirection;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WebSocketMessage {
    pub(crate) pid: u32,
    pub(crate) stream_key: u64,
    pub(crate) direction: CaptureDirection,
    pub(crate) path: String,
    pub(crate) compressed: bool,
    pub(crate) wire_bytes: usize,
    pub(crate) payload: WebSocketPayload,
}

impl WebSocketMessage {
    pub(crate) fn text(&self) -> Option<&str> {
        match &self.payload {
            WebSocketPayload::Text(text) => Some(text),
            WebSocketPayload::Binary(_) => None,
        }
    }

    pub(crate) fn payload_bytes(&self) -> usize {
        match &self.payload {
            WebSocketPayload::Text(text) => text.len(),
            WebSocketPayload::Binary(bytes) => bytes.len(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WebSocketPayload {
    Text(String),
    Binary(Vec<u8>),
}

#[derive(Clone, Debug)]
pub(super) struct NegotiatedExtensions {
    pub(super) permessage_deflate: bool,
    pub(super) client_no_context_takeover: bool,
    pub(super) server_no_context_takeover: bool,
}
