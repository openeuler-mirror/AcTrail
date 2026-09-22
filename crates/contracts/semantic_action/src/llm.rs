//! LLM provider protocol parsing contracts.

use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LlmProviderMatch {
    NoMatch,
    Plausible,
    Strong,
}

/// Content retained by a response parser; recognition and completion are always parsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LlmResponseRetention {
    pub content: bool,
    pub tool_calls: bool,
    pub usage: bool,
}

impl Default for LlmResponseRetention {
    fn default() -> Self {
        Self {
            content: true,
            tool_calls: true,
            usage: true,
        }
    }
}

/// Observed provider termination. Transport closure is tracked separately.
/// Failure takes precedence if a batch also carries a normal completion marker.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LlmResponseTermination {
    Completed,
    Incomplete,
    Failed,
}

impl LlmResponseTermination {
    pub fn combine(current: Option<Self>, observed: Option<Self>) -> Option<Self> {
        current.max(observed)
    }

    pub fn is_failure(self) -> bool {
        matches!(self, Self::Failed | Self::Incomplete)
    }
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

    fn parse_json_response(
        &self,
        input: LlmJsonResponseInput<'_>,
        retention: LlmResponseRetention,
    ) -> Option<LlmParsedResponse>;

    fn parse_sse_response(
        &self,
        input: LlmSseResponseInput<'_>,
        retention: LlmResponseRetention,
    ) -> Option<LlmParsedResponse>;

    fn parse_sse_event(
        &self,
        event: LlmSseEvent<'_>,
        retention: LlmResponseRetention,
    ) -> LlmParsedSseEvent;

    fn new_stream_parser(
        &self,
        retention: LlmResponseRetention,
    ) -> Box<dyn LlmProviderResponseStreamParser + Send>;
}

pub trait LlmProviderResponseStreamParser {
    fn observe_event(&mut self, event: LlmSseEvent<'_>) -> LlmParsedSseEvent;

    fn finish(&mut self) -> Option<LlmParsedResponse>;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LlmParsedResponse {
    pub provider_id: &'static str,
    pub model: Option<String>,
    pub content_text: Option<String>,
    pub reasoning_text: Option<String>,
    pub tool_calls: Vec<LlmToolCall>,
    pub token_usage: Option<LlmTokenUsage>,
    pub chunk_count: usize,
    pub termination: Option<LlmResponseTermination>,
    pub stream: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LlmParsedSseEvent {
    /// Text/reasoning chunks observed on the wire, including discarded content.
    pub text_chunk_count: usize,
    /// Recognized response content or tool evidence, independent of retention.
    pub response_observed: bool,
    pub model: Option<String>,
    pub content_text: Option<String>,
    pub reasoning_text: Option<String>,
    pub tool_calls: Vec<LlmToolCall>,
    pub termination: Option<LlmResponseTermination>,
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
    /// Tokens written into the provider's prompt cache by this request.
    /// Distinct from `prompt_cache_miss_tokens`: a miss is prompt content
    /// billed at full rate because the cache did not serve it, while a cache
    /// creation is content the provider stored for later reuse. Anthropic
    /// reports the latter as `cache_creation_input_tokens` and has no field
    /// meaning the former.
    pub cache_creation_tokens: Option<u64>,
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
