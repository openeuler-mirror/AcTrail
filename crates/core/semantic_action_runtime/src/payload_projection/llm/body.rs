//! LLM response body and SSE framing adapter.

use config_core::daemon::SseEventContentRetention;
use semantic_action::{
    LlmJsonResponseInput, LlmParsedResponse, LlmParsedSseEvent, LlmSseEvent as ProviderSseEvent,
    LlmSseResponseInput, LlmTokenUsage,
};
use serde_json::{Map, Value};

use super::codec::{LlmCodecRegistry, NormalizedSseEvent, SseCodecEvent};
use super::provider::{parse_json_response, parse_sse_response, tool_calls_json};

const SSE_DONE_MARKER: &str = "[DONE]";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LlmResponseBody {
    pub(super) provider_id: String,
    pub(super) json_valid: bool,
    pub(super) model: Option<String>,
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
    pub(super) raw_data: String,
    pub(super) model: Option<String>,
    pub(super) content_text: Option<String>,
    pub(super) reasoning_text: Option<String>,
    pub(super) tool_calls_json: Option<String>,
    pub(super) done: bool,
    pub(super) finish_reason: Option<String>,
    pub(super) has_content_delta: bool,
    pub(super) has_reasoning_delta: bool,
    pub(super) has_tool_delta: bool,
}

pub(super) fn parse_llm_response_body(
    body: &[u8],
    codecs: &LlmCodecRegistry,
) -> Option<LlmResponseBody> {
    LlmResponseBodyParser { codecs }.parse(body)
}

pub(super) fn sse_events_json(
    events: &[SseEvent],
    content: SseEventContentRetention,
) -> Option<String> {
    let values = match content {
        SseEventContentRetention::None => return None,
        SseEventContentRetention::Parsed => {
            events.iter().map(sse_event_json_value).collect::<Vec<_>>()
        }
        SseEventContentRetention::Raw => events
            .iter()
            .map(raw_sse_event_json_value)
            .collect::<Vec<_>>(),
    };
    (!values.is_empty()).then(|| Value::Array(values).to_string())
}

struct LlmResponseBodyParser<'a> {
    codecs: &'a LlmCodecRegistry,
}

impl LlmResponseBodyParser<'_> {
    fn parse(&self, body: &[u8]) -> Option<LlmResponseBody> {
        let text = String::from_utf8_lossy(body).into_owned();
        if let Some(sse) = self.parse_sse_response_body(&text) {
            return Some(sse);
        }
        let json = serde_json::from_slice::<Value>(body).ok();
        let value = json.as_ref()?;
        let parsed = parse_json_response(LlmJsonResponseInput {
            text: &text,
            json: value,
        })?;
        Some(response_body(true, parsed, Vec::new()))
    }

    fn parse_sse_response_body(&self, text: &str) -> Option<LlmResponseBody> {
        let raw_events = parse_sse_events(text);
        if raw_events.is_empty() {
            return None;
        }
        if let Some((provider_id, normalized)) = self.normalized_sse_events(&raw_events) {
            return decoded_sse_response(text, raw_events, provider_id, normalized);
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
        Some(response_body(false, parsed.response, events))
    }

    fn normalized_sse_events(
        &self,
        raw_events: &[SseCodecEvent],
    ) -> Option<(Option<String>, Vec<NormalizedSseEvent>)> {
        let mut decoded_any = false;
        let mut provider_id = None;
        let mut normalized = Vec::with_capacity(raw_events.len());
        for event in raw_events {
            if let Some(decoded) = self.codecs.decode_sse_event(event) {
                let data = String::from_utf8(decoded.body).ok()?;
                let trimmed = data.trim();
                provider_id = provider_id.or(decoded.provider_id);
                normalized.push(NormalizedSseEvent {
                    index: event.index,
                    event_type: event.event_type.clone(),
                    id: event.id.clone(),
                    json: serde_json::from_str::<Value>(trimmed).ok(),
                    done_marker: trimmed == SSE_DONE_MARKER,
                    data,
                });
                decoded_any = true;
            } else {
                normalized.push(normalized_event_from_raw(event));
            }
        }
        decoded_any.then_some((provider_id, normalized))
    }
}

fn decoded_sse_response(
    text: &str,
    raw_events: Vec<SseCodecEvent>,
    provider_id: Option<String>,
    normalized: Vec<NormalizedSseEvent>,
) -> Option<LlmResponseBody> {
    let provider_events = normalized
        .iter()
        .map(provider_normalized_sse_event)
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
    let mut body = response_body(false, parsed.response, events);
    if let Some(provider_id) = provider_id {
        body.provider_id = provider_id;
    }
    Some(body)
}

fn response_body(
    json_valid: bool,
    parsed: LlmParsedResponse,
    sse_events: Vec<SseEvent>,
) -> LlmResponseBody {
    let tool_calls_json = tool_calls_json(&parsed.tool_calls);
    LlmResponseBody {
        provider_id: parsed.provider_id.to_string(),
        json_valid,
        model: parsed.model,
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

fn parse_sse_events(text: &str) -> Vec<SseCodecEvent> {
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
) -> SseCodecEvent {
    let trimmed = data.trim();
    SseCodecEvent {
        index,
        event_type,
        id,
        json: serde_json::from_str::<Value>(trimmed).ok(),
        done_marker: trimmed == SSE_DONE_MARKER,
        data,
    }
}

fn provider_sse_event(event: &SseCodecEvent) -> ProviderSseEvent<'_> {
    ProviderSseEvent {
        index: event.index,
        event_type: event.event_type.as_deref(),
        id: event.id.as_deref(),
        data: &event.data,
        json: event.json.as_ref(),
        done_marker: event.done_marker,
    }
}

fn provider_normalized_sse_event(event: &NormalizedSseEvent) -> ProviderSseEvent<'_> {
    ProviderSseEvent {
        index: event.index,
        event_type: event.event_type.as_deref(),
        id: event.id.as_deref(),
        data: &event.data,
        json: event.json.as_ref(),
        done_marker: event.done_marker,
    }
}

fn normalized_event_from_raw(event: &SseCodecEvent) -> NormalizedSseEvent {
    NormalizedSseEvent {
        index: event.index,
        event_type: event.event_type.clone(),
        id: event.id.clone(),
        data: event.data.clone(),
        json: event.json.clone(),
        done_marker: event.done_marker,
    }
}

fn sse_event(raw: SseCodecEvent, parsed: LlmParsedSseEvent) -> SseEvent {
    let tool_calls_json = tool_calls_json(&parsed.tool_calls);
    SseEvent {
        index: raw.index,
        event_type: raw.event_type,
        id: raw.id,
        raw_data: raw.data,
        model: parsed.model,
        has_content_delta: parsed.content_text.is_some(),
        has_reasoning_delta: parsed.reasoning_text.is_some(),
        has_tool_delta: tool_calls_json.is_some(),
        content_text: parsed.content_text,
        reasoning_text: parsed.reasoning_text,
        tool_calls_json,
        done: parsed.done,
        finish_reason: parsed.finish_reason,
    }
}

fn sse_event_json_value(event: &SseEvent) -> Value {
    let mut object = Map::new();
    object.insert(
        "index".to_string(),
        Value::Number(serde_json::Number::from(event.index as u64)),
    );
    if let Some(event_type) = &event.event_type {
        object.insert("event_type".to_string(), Value::String(event_type.clone()));
    }
    if let Some(id) = &event.id {
        object.insert("id".to_string(), Value::String(id.clone()));
    }
    if let Some(model) = &event.model {
        object.insert("model".to_string(), Value::String(model.clone()));
    }
    if let Some(content_text) = &event.content_text {
        object.insert(
            "content_text".to_string(),
            Value::String(content_text.clone()),
        );
    }
    if let Some(reasoning_text) = &event.reasoning_text {
        object.insert(
            "reasoning_text".to_string(),
            Value::String(reasoning_text.clone()),
        );
    }
    if let Some(tool_calls_json) = &event.tool_calls_json
        && let Ok(tool_calls) = serde_json::from_str::<Value>(tool_calls_json)
    {
        object.insert("tool_calls".to_string(), tool_calls);
    }
    object.insert("done".to_string(), Value::Bool(event.done));
    if let Some(finish_reason) = &event.finish_reason {
        object.insert(
            "finish_reason".to_string(),
            Value::String(finish_reason.clone()),
        );
    }
    Value::Object(object)
}

fn raw_sse_event_json_value(event: &SseEvent) -> Value {
    let mut object = Map::new();
    object.insert(
        "index".to_string(),
        Value::Number(serde_json::Number::from(event.index as u64)),
    );
    if let Some(event_type) = &event.event_type {
        object.insert("event_type".to_string(), Value::String(event_type.clone()));
    }
    if let Some(id) = &event.id {
        object.insert("id".to_string(), Value::String(id.clone()));
    }
    object.insert("data".to_string(), Value::String(event.raw_data.clone()));
    object.insert("done".to_string(), Value::Bool(event.done));
    Value::Object(object)
}
