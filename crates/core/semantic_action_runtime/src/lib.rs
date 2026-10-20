//! Runtime projection from low-level facts into semantic actions.

mod http_observation;
pub mod live;
mod llm_pipeline;

pub use http_observation::{
    Http1AbortReason, Http1MessageEnd, Http1MessageEvent, Http1MessageHead, Http1MessageSource,
    Http2FrameObservation,
};

pub use live::LiveSemanticActionRuntime;
pub use llm_pipeline::{
    LlmCodecDecoded, LlmCodecOutcome, LlmCodecPlugin, LlmCodecPluginStatus, LlmCodecRegistry,
    LlmCodecRequest, LlmCodecSseEvent,
};
