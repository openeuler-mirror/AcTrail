mod adapter;
mod connection;
mod framing;
mod handshake;

pub(in crate::llm_pipeline) use adapter::{
    WebSocketExchangeStreamPrefix, WebSocketLlmAdapter, WebSocketLlmObservation,
};
