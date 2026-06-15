//! LLM response body and SSE framing adapter.

use semantic_action::{
    LlmJsonResponseInput, LlmParsedResponse, LlmParsedSseEvent, LlmSseEvent as ProviderSseEvent,
    LlmSseResponseInput, LlmTokenUsage,
};
use serde_json::Value;

use super::provider::{parse_json_response, parse_sse_response, tool_calls_json};

const SSE_DONE_MARKER: &str = "[DONE]";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LlmResponseBody {
    pub(super) text: String,
    pub(super) json: Option<Value>,
    pub(super) model: Option<String>,
    pub(super) output_text: Option<String>,
    pub(super) content_text: Option<String>,
    pub(super) reasoning_text: Option<String>,
    pub(super) tool_calls_json: Option<String>,
    pub(super) token_usage: Option<LlmTokenUsage>,
    pub(super) chunk_count: usize,
    pub(super) done: bool,
    pub(super) stream: bool,
    pub(super) sse_events: Vec<SseEvent>,
}

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

#[derive(Clone, Debug, Eq, PartialEq)]
struct RawSseEvent {
    index: usize,
    event_type: Option<String>,
    id: Option<String>,
    data: String,
    json: Option<Value>,
    done_marker: bool,
}

pub(super) fn parse_llm_response_body(body: &[u8]) -> Option<LlmResponseBody> {
    let text = String::from_utf8_lossy(body).into_owned();
    if let Some(sse) = parse_sse_response_body(&text) {
        return Some(sse);
    }
    let json = serde_json::from_slice::<Value>(body).ok();
    let value = json.as_ref()?;
    let parsed = parse_json_response(LlmJsonResponseInput {
        text: &text,
        json: value,
    })?;
    Some(response_body(text, json, parsed, Vec::new()))
}

fn parse_sse_response_body(text: &str) -> Option<LlmResponseBody> {
    let raw_events = parse_sse_events(text);
    if raw_events.is_empty() {
        return None;
    }
    let provider_events = raw_events
        .iter()
        .map(provider_sse_event)
        .collect::<Vec<_>>();
    let parsed = parse_sse_response(LlmSseResponseInput {
        text,
        events: &provider_events,
    })?;
    let events = raw_events
        .into_iter()
        .zip(parsed.events)
        .map(|(raw, parsed)| sse_event(raw, parsed))
        .collect();
    Some(response_body(
        text.to_string(),
        None,
        parsed.response,
        events,
    ))
}

fn response_body(
    text: String,
    json: Option<Value>,
    parsed: LlmParsedResponse,
    sse_events: Vec<SseEvent>,
) -> LlmResponseBody {
    let tool_calls_json = tool_calls_json(&parsed.tool_calls);
    LlmResponseBody {
        text,
        json,
        model: parsed.model,
        output_text: parsed.output_text,
        content_text: parsed.content_text,
        reasoning_text: parsed.reasoning_text,
        tool_calls_json,
        token_usage: parsed.token_usage,
        chunk_count: parsed.chunk_count,
        done: parsed.done,
        stream: parsed.stream,
        sse_events,
    }
}

fn parse_sse_events(text: &str) -> Vec<RawSseEvent> {
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
            items.push(raw_sse_event(items.len(), event_type, id, data));
        }
    }
    items
}

fn raw_sse_event(
    index: usize,
    event_type: Option<String>,
    id: Option<String>,
    data: String,
) -> RawSseEvent {
    let trimmed = data.trim();
    RawSseEvent {
        index,
        event_type,
        id,
        json: serde_json::from_str::<Value>(trimmed).ok(),
        done_marker: trimmed == SSE_DONE_MARKER,
        data,
    }
}

fn provider_sse_event(event: &RawSseEvent) -> ProviderSseEvent<'_> {
    ProviderSseEvent {
        index: event.index,
        event_type: event.event_type.as_deref(),
        id: event.id.as_deref(),
        data: &event.data,
        json: event.json.as_ref(),
        done_marker: event.done_marker,
    }
}

fn sse_event(raw: RawSseEvent, parsed: LlmParsedSseEvent) -> SseEvent {
    SseEvent {
        index: raw.index,
        event_type: raw.event_type,
        id: raw.id,
        data: raw.data,
        json: raw.json,
        model: parsed.model,
        content_text: parsed.content_text,
        reasoning_text: parsed.reasoning_text,
        tool_calls_json: tool_calls_json(&parsed.tool_calls),
        done: parsed.done,
        finish_reason: parsed.finish_reason,
    }
}
