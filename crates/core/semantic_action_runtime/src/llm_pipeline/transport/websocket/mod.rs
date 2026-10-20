mod adapter;
mod connection;
mod deflate;
mod framing;
mod handshake;
mod model;
pub(in crate::llm_pipeline) use adapter::WebSocketTransport;
pub(in crate::llm_pipeline) use model::{
    WebSocketAbortReason, WebSocketConnectionKey, WebSocketMessageEvent, WebSocketMessageHead,
};
