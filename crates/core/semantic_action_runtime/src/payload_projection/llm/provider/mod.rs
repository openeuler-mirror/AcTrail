//! Provider-specific LLM response protocol parsers.

mod anthropic;
mod anthropic_stream;
mod common;
mod generic_request;
mod openai;
mod openai_responses;
mod registry;
mod request_registry;
mod structured_json_sse;

pub(super) use common::{extract_token_usage, tool_calls_json};
pub(super) use registry::{new_sse_stream_parser, parse_json_response, parse_sse_response};
pub(super) use request_registry::{LlmRequestParserInput, parse_json_request};
