//! WebSocket assembly over captured TLS plaintext events.

mod assembler;
mod deflate;
mod frame;
mod message;
mod model;

pub(crate) use assembler::WebSocketAssembler;
pub(crate) use model::{WebSocketMessage, WebSocketPayload};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WebSocketConfig {
    pub(crate) max_frame_buffer_bytes: usize,
    pub(crate) max_message_bytes: usize,
    pub(crate) max_decoded_bytes: usize,
}
