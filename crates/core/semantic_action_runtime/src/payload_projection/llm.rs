//! LLM semantic action projection from retained plaintext payloads.

mod body;
mod evidence;
mod live_projection;
mod provider;
mod request;
mod response;
mod response_support;
mod sse;
mod stream;

pub(crate) use live_projection::{
    LiveLlmProjection, live_llm_http_response_message_len, live_llm_request_message_len,
    project_live_llm_request_message, project_live_llm_response_message,
};
pub(crate) use stream::PayloadStreamGroupKey;
