//! LLM semantic action projection from retained plaintext payloads.

mod body;
mod codec;
mod evidence;
mod live_projection;
mod provider;
mod request;
mod request_blocks;
mod response;
mod response_support;
mod sse;
mod stream;

pub use codec::LlmCodecRegistry;
pub(crate) use live_projection::{
    LiveLlmProjection, live_llm_http_response_message_len, live_llm_request_message_len,
    live_llm_request_prefix_skip_len, live_llm_request_stream_id_hint,
    project_live_llm_request_message, project_live_llm_response_message,
};
pub use plugin_system::{
    LlmCodecDecoded, LlmCodecOutcome, LlmCodecPlugin, LlmCodecPluginStatus, LlmCodecRequest,
    LlmCodecSseEvent,
};
pub(crate) use stream::PayloadStreamGroupKey;
