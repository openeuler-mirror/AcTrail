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
mod stream;

pub(crate) use body::IncrementalSseCache;
pub use codec::LlmCodecRegistry;
pub(crate) use live_projection::{
    LiveLlmProjection, LiveLlmResponseMessage, live_llm_request_message_len,
    live_llm_request_prefix_skip_len, project_http2_stream_request, project_http2_stream_response,
    project_live_llm_request_message, project_live_llm_response_message, semantic_payload_draft,
};
pub use plugin_system::{
    LlmCodecDecoded, LlmCodecOutcome, LlmCodecPlugin, LlmCodecPluginStatus, LlmCodecRequest,
    LlmCodecSseEvent,
};
pub(crate) use request::{ProjectedLlmRequestHistory, ProviderContextReference};
pub(crate) use request_blocks::{HistoryAtom, TrajectoryHistoryProjection};
pub(crate) use response::InFlightResponse;
pub(crate) use response::ProjectedProviderResponseId;
pub(crate) use stream::PayloadStreamGroupKey;
