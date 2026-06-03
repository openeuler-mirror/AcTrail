//! LLM text projection over parsed SSE frames.

mod inbound;
mod model;
mod outbound;
mod router;

pub(crate) use model::{LlmDelta, LlmMessage, LlmOutput, LlmRequest};
pub(crate) use router::LlmProjector;
