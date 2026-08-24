pub(super) mod buffer;
pub(super) mod evidence;
pub(in crate::llm_pipeline) mod http1;
pub(in crate::llm_pipeline) mod http2;
mod message;
pub(super) mod websocket;

pub(crate) use message::{HttpRequestParts, HttpResponseParts};
