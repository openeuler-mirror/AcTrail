//! Provider-specific LLM response protocol parsers.

mod anthropic;
mod anthropic_stream;
mod common;
mod generic_request;
mod openai;
mod registry;
mod request_registry;
mod structured_json_sse;

pub(super) use common::tool_calls_json;
pub(super) use registry::{parse_json_response, parse_sse_response};
pub(super) use request_registry::{LlmRequestParserInput, parse_json_request};
