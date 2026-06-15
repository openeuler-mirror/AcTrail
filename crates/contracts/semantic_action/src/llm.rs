//! LLM provider protocol parsing contracts.

use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LlmProviderMatch {
    NoMatch,
    Plausible,
    Strong,
}

#[derive(Clone, Copy, Debug)]
pub struct LlmJsonResponseInput<'a> {
    pub text: &'a str,
    pub json: &'a Value,
}

#[derive(Clone, Copy, Debug)]
pub struct LlmSseEvent<'a> {
    pub index: usize,
    pub event_type: Option<&'a str>,
    pub id: Option<&'a str>,
    pub data: &'a str,
    pub json: Option<&'a Value>,
    pub done_marker: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct LlmSseResponseInput<'a> {
    pub text: &'a str,
    pub events: &'a [LlmSseEvent<'a>],
}

pub trait LlmProviderResponseParser: Send + Sync {
    fn provider_id(&self) -> &'static str;

    fn match_json_response(&self, input: LlmJsonResponseInput<'_>) -> LlmProviderMatch;

    fn match_sse_response(&self, input: LlmSseResponseInput<'_>) -> LlmProviderMatch;

    fn parse_json_response(&self, input: LlmJsonResponseInput<'_>) -> Option<LlmParsedResponse>;

    fn parse_sse_response(&self, input: LlmSseResponseInput<'_>) -> Option<LlmParsedResponse>;

    fn parse_sse_event(&self, event: LlmSseEvent<'_>) -> LlmParsedSseEvent;

    fn new_stream_parser(&self) -> Box<dyn LlmProviderResponseStreamParser + Send>;
}

pub trait LlmProviderResponseStreamParser {
    fn observe_event(&mut self, event: LlmSseEvent<'_>) -> LlmParsedSseEvent;

    fn finish(&mut self) -> Option<LlmParsedResponse>;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LlmParsedResponse {
    pub provider_id: &'static str,
    pub model: Option<String>,
    pub output_text: Option<String>,
    pub content_text: Option<String>,
    pub reasoning_text: Option<String>,
    pub tool_calls: Vec<LlmToolCall>,
    pub token_usage: Option<LlmTokenUsage>,
    pub chunk_count: usize,
    pub done: bool,
    pub stream: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LlmParsedSseEvent {
    pub model: Option<String>,
    pub content_text: Option<String>,
    pub reasoning_text: Option<String>,
    pub tool_calls: Vec<LlmToolCall>,
    pub done: bool,
    pub finish_reason: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LlmTokenUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cached_prompt_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub prompt_cache_hit_tokens: Option<u64>,
    pub prompt_cache_miss_tokens: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LlmToolCall {
    pub index: Option<u64>,
    pub id: Option<String>,
    pub kind: Option<String>,
    pub function: Option<LlmToolFunction>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LlmToolFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
    pub arguments_json: Option<Value>,
}
