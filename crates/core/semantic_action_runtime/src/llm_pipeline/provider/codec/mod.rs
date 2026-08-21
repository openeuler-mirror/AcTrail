mod adapter;

pub use adapter::LlmCodecRegistry;
pub(in crate::llm_pipeline) use adapter::{NormalizedSseEvent, SseCodecEvent};
pub use plugin_system::{
    LlmCodecDecoded, LlmCodecOutcome, LlmCodecPlugin, LlmCodecPluginStatus, LlmCodecRequest,
    LlmCodecSseEvent,
};
