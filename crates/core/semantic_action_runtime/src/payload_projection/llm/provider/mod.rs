//! Provider-specific LLM response protocol parsers.

mod anthropic;
mod anthropic_stream;
mod common;
mod openai;
mod registry;

pub(super) use common::tool_calls_json;
pub(super) use registry::{parse_json_response, parse_sse_response};
