//! LLM response body and SSE parsing.

use super::tool_calls::{
    assembled_tool_calls_json, assembled_tool_calls_json_from_values, collect_tool_call_deltas_json,
};
use serde_json::Value;

const SSE_DONE_MARKER: &str = "[DONE]";
const CLAUDE_MESSAGE_STOP: &str = "message_stop";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LlmResponseBody {
    pub(super) text: String,
    pub(super) json: Option<Value>,
    pub(super) model: Option<String>,
    pub(super) output_text: Option<String>,
    pub(super) content_text: Option<String>,
    pub(super) reasoning_text: Option<String>,
    pub(super) tool_calls_json: Option<String>,
    pub(super) token_usage: Option<TokenUsage>,
    pub(super) chunk_count: usize,
    pub(super) done: bool,
    pub(super) stream: bool,
    pub(super) sse_events: Vec<SseEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TokenUsage {
    pub(super) prompt_tokens: Option<u64>,
    pub(super) completion_tokens: Option<u64>,
    pub(super) total_tokens: Option<u64>,
    pub(super) cached_prompt_tokens: Option<u64>,
    pub(super) reasoning_tokens: Option<u64>,
    pub(super) prompt_cache_hit_tokens: Option<u64>,
    pub(super) prompt_cache_miss_tokens: Option<u64>,
}

trait TokenUsageExtractor {
    fn extract(&self, value: &Value) -> Option<TokenUsage>;
}

struct OpenAiCompatibleTokenUsageExtractor;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SseEvent {
    pub(super) index: usize,
    pub(super) event_type: Option<String>,
    pub(super) id: Option<String>,
    pub(super) data: String,
    pub(super) json: Option<Value>,
    pub(super) model: Option<String>,
    pub(super) content_text: Option<String>,
    pub(super) reasoning_text: Option<String>,
    pub(super) tool_calls_json: Option<String>,
    pub(super) done: bool,
    pub(super) finish_reason: Option<String>,
}

pub(super) fn parse_llm_response_body(body: &[u8]) -> Option<LlmResponseBody> {
    let text = String::from_utf8_lossy(body).into_owned();
    if let Some(sse) = parse_sse_response_body(&text) {
        return Some(sse);
    }
    let json = serde_json::from_slice::<Value>(body).ok();
    let value = json.as_ref()?;
    if !json_value_is_llm_response(value) {
        return None;
    }
    let texts = extract_response_texts(value);
    let tool_calls_json = assembled_tool_calls_json(value);
    let token_usage = extract_token_usage(value);
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    Some(LlmResponseBody {
        text,
        json,
        model,
        chunk_count: texts.chunk_count(),
        output_text: texts.output_text(),
        content_text: texts.content_text,
        reasoning_text: texts.reasoning_text,
        tool_calls_json,
        token_usage,
        done: true,
        stream: false,
        sse_events: Vec::new(),
    })
}

fn parse_sse_response_body(text: &str) -> Option<LlmResponseBody> {
    let sse_events = parse_sse_events(text);
    if sse_events.is_empty() {
        return None;
    }
    let mut content_chunks = Vec::new();
    let mut reasoning_chunks = Vec::new();
    let mut model = None;
    let mut done = false;
    for event in &sse_events {
        if event.done {
            done = true;
        }
        if model.is_none() {
            model = event.model.clone();
        }
        if let Some(text) = &event.content_text {
            content_chunks.push(text.clone());
        }
        if let Some(text) = &event.reasoning_text {
            reasoning_chunks.push(text.clone());
        }
    }
    let tool_calls_json = assembled_tool_calls_json_from_values(
        sse_events.iter().filter_map(|event| event.json.as_ref()),
    );
    let token_usage =
        extract_token_usage_from_values(sse_events.iter().filter_map(|event| event.json.as_ref()));
    if content_chunks.is_empty()
        && reasoning_chunks.is_empty()
        && tool_calls_json.is_none()
        && !done
    {
        return None;
    }
    let content_text = (!content_chunks.is_empty()).then(|| content_chunks.join(""));
    let reasoning_text = (!reasoning_chunks.is_empty()).then(|| reasoning_chunks.join(""));
    let output_text = content_text.clone().or_else(|| reasoning_text.clone());
    Some(LlmResponseBody {
        text: text.to_string(),
        json: None,
        model,
        output_text,
        content_text,
        reasoning_text,
        tool_calls_json,
        token_usage,
        chunk_count: content_chunks.len() + reasoning_chunks.len(),
        done,
        stream: true,
        sse_events,
    })
}

fn parse_sse_events(text: &str) -> Vec<SseEvent> {
    let mut items = Vec::new();
    for block in text.split("\n\n").filter(|block| !block.trim().is_empty()) {
        let mut data_lines = Vec::new();
        let mut event_type = None;
        let mut id = None;
        for line in block.lines() {
            let line = line.trim_end_matches('\r');
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            match name.trim().to_ascii_lowercase().as_str() {
                "data" => data_lines.push(value.trim_start()),
                "event" => event_type = Some(value.trim().to_string()),
                "id" => id = Some(value.trim().to_string()),
                _ => {}
            }
        }
        if !data_lines.is_empty() {
            let data = data_lines.join("\n");
            items.push(sse_event(items.len(), event_type, id, data));
        }
    }
    items
}

fn sse_event(
    index: usize,
    event_type: Option<String>,
    id: Option<String>,
    data: String,
) -> SseEvent {
    let trimmed = data.trim();
    if trimmed == SSE_DONE_MARKER {
        return SseEvent {
            index,
            event_type,
            id,
            data,
            json: None,
            model: None,
            content_text: None,
            reasoning_text: None,
            tool_calls_json: None,
            done: true,
            finish_reason: None,
        };
    }
    let json = serde_json::from_str::<Value>(trimmed).ok();
    let texts = json
        .as_ref()
        .map(extract_response_texts)
        .unwrap_or_default();
    let model = json
        .as_ref()
        .and_then(|value| value.get("model"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let tool_calls_json = json.as_ref().and_then(collect_tool_call_deltas_json);
    let done = json.as_ref().is_some_and(|value| {
        value.get("type").and_then(Value::as_str) == Some(CLAUDE_MESSAGE_STOP)
    });
    let finish_reason = json.as_ref().and_then(extract_finish_reason);
    SseEvent {
        index,
        event_type,
        id,
        data: data.clone(),
        json,
        model,
        content_text: texts.content_text,
        reasoning_text: texts.reasoning_text,
        tool_calls_json,
        done,
        finish_reason,
    }
}

fn json_value_is_llm_response(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.contains_key("model")
        && (object.contains_key("choices")
            || object.contains_key("output")
            || object.contains_key("content"))
}

fn extract_token_usage(value: &Value) -> Option<TokenUsage> {
    OpenAiCompatibleTokenUsageExtractor.extract(value)
}

fn extract_token_usage_from_values<'a>(
    values: impl IntoIterator<Item = &'a Value>,
) -> Option<TokenUsage> {
    values.into_iter().filter_map(extract_token_usage).last()
}

impl TokenUsageExtractor for OpenAiCompatibleTokenUsageExtractor {
    fn extract(&self, value: &Value) -> Option<TokenUsage> {
        let usage = value.get("usage")?.as_object()?;
        let prompt_tokens = token_count(usage.get("prompt_tokens"))
            .or_else(|| token_count(usage.get("input_tokens")));
        let completion_tokens = token_count(usage.get("completion_tokens"))
            .or_else(|| token_count(usage.get("output_tokens")));
        let total_tokens = token_count(usage.get("total_tokens"));
        let cached_prompt_tokens = usage
            .get("prompt_tokens_details")
            .and_then(Value::as_object)
            .and_then(|details| token_count(details.get("cached_tokens")));
        let reasoning_tokens = usage
            .get("completion_tokens_details")
            .and_then(Value::as_object)
            .and_then(|details| token_count(details.get("reasoning_tokens")));
        let prompt_cache_hit_tokens = token_count(usage.get("prompt_cache_hit_tokens"));
        let prompt_cache_miss_tokens = token_count(usage.get("prompt_cache_miss_tokens"));
        let usage = TokenUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens,
            cached_prompt_tokens,
            reasoning_tokens,
            prompt_cache_hit_tokens,
            prompt_cache_miss_tokens,
        };
        usage.has_any_count().then_some(usage)
    }
}

impl TokenUsage {
    fn has_any_count(&self) -> bool {
        self.prompt_tokens
            .or(self.completion_tokens)
            .or(self.total_tokens)
            .or(self.cached_prompt_tokens)
            .or(self.reasoning_tokens)
            .or(self.prompt_cache_hit_tokens)
            .or(self.prompt_cache_miss_tokens)
            .is_some()
    }
}

fn token_count(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ResponseTexts {
    content_text: Option<String>,
    reasoning_text: Option<String>,
}

impl ResponseTexts {
    fn output_text(&self) -> Option<String> {
        self.content_text
            .clone()
            .or_else(|| self.reasoning_text.clone())
    }

    fn chunk_count(&self) -> usize {
        usize::from(self.content_text.is_some()) + usize::from(self.reasoning_text.is_some())
    }
}

fn extract_response_texts(value: &Value) -> ResponseTexts {
    let mut content_chunks = Vec::new();
    let mut reasoning_chunks = Vec::new();
    collect_response_text(value, &mut content_chunks, &mut reasoning_chunks);
    ResponseTexts {
        content_text: (!content_chunks.is_empty()).then(|| content_chunks.join("")),
        reasoning_text: (!reasoning_chunks.is_empty()).then(|| reasoning_chunks.join("")),
    }
}

fn collect_response_text(
    value: &Value,
    content_chunks: &mut Vec<String>,
    reasoning_chunks: &mut Vec<String>,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_response_text(item, content_chunks, reasoning_chunks);
            }
        }
        Value::Object(object) => {
            for key in ["content", "text", "output_text"] {
                if let Some(text) = object.get(key).and_then(Value::as_str) {
                    push_non_empty_text(content_chunks, text);
                }
            }
            if let Some(text) = object.get("reasoning_content").and_then(Value::as_str) {
                push_non_empty_text(reasoning_chunks, text);
            }
            if let Some(text) = object.get("thinking").and_then(Value::as_str) {
                push_non_empty_text(reasoning_chunks, text);
            }
            for key in ["content", "message", "delta", "choices", "output"] {
                if let Some(child) = object.get(key) {
                    collect_response_text(child, content_chunks, reasoning_chunks);
                }
            }
        }
        _ => {}
    }
}

fn push_non_empty_text(chunks: &mut Vec<String>, text: &str) {
    if !text.is_empty() {
        chunks.push(text.to_string());
    }
}

fn extract_finish_reason(value: &Value) -> Option<String> {
    match value {
        Value::Array(items) => items.iter().find_map(extract_finish_reason),
        Value::Object(object) => object
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .or_else(|| object.get("choices").and_then(extract_finish_reason)),
        _ => None,
    }
}
