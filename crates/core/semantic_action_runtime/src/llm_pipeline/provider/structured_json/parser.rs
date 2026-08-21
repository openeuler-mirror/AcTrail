//! Structured JSON request with structured SSE response parser.

use semantic_action::{
    LlmJsonResponseInput, LlmParsedResponse, LlmParsedSseEvent, LlmProviderMatch,
    LlmProviderResponseParser, LlmProviderResponseStreamParser, LlmSseEvent, LlmSseResponseInput,
    LlmTokenUsage,
};
use serde_json::{Map, Value};

use crate::llm_pipeline::provider::driver::{
    ParsedSseResponseAccumulator, push_non_empty_text, token_usage_from_map,
};
use crate::llm_pipeline::provider::registry::{
    LlmRequestParser, LlmRequestParserInput, ParsedLlmRequest,
};

pub(in crate::llm_pipeline) const STRUCTURED_JSON_SSE_PROTOCOL_ID: &str = "structured-json-sse";

#[derive(Default)]
pub(in crate::llm_pipeline) struct StructuredJsonSseParser;

impl LlmRequestParser for StructuredJsonSseParser {
    fn classifier_id(&self) -> &'static str {
        STRUCTURED_JSON_SSE_PROTOCOL_ID
    }

    fn match_json_request(&self, input: &LlmRequestParserInput<'_>) -> LlmProviderMatch {
        if structured_json_sse_request(input) {
            LlmProviderMatch::Strong
        } else {
            LlmProviderMatch::NoMatch
        }
    }

    fn parse_json_request(&self, input: &LlmRequestParserInput<'_>) -> Option<ParsedLlmRequest> {
        if self.match_json_request(input) == LlmProviderMatch::NoMatch {
            return None;
        }
        Some(ParsedLlmRequest {
            classifier_id: self.classifier_id(),
            protocol_id: Some(STRUCTURED_JSON_SSE_PROTOCOL_ID),
            model: request_model(input.json).map(ToString::to_string),
        })
    }
}

impl LlmProviderResponseParser for StructuredJsonSseParser {
    fn provider_id(&self) -> &'static str {
        STRUCTURED_JSON_SSE_PROTOCOL_ID
    }

    fn match_json_response(&self, _input: LlmJsonResponseInput<'_>) -> LlmProviderMatch {
        LlmProviderMatch::NoMatch
    }

    fn match_sse_response(&self, input: LlmSseResponseInput<'_>) -> LlmProviderMatch {
        let evidence = StructuredJsonSseEvidence::from_input(input);
        if evidence.is_strong() {
            LlmProviderMatch::Strong
        } else if evidence.has_output_delta {
            // An output event is independently sufficient for incremental
            // recognition when metadata and output arrive in different reads
            // or on opposite sides of the classifier's soft sniff window.
            LlmProviderMatch::Plausible
        } else {
            LlmProviderMatch::NoMatch
        }
    }

    fn parse_json_response(&self, _input: LlmJsonResponseInput<'_>) -> Option<LlmParsedResponse> {
        None
    }

    fn parse_sse_response(&self, input: LlmSseResponseInput<'_>) -> Option<LlmParsedResponse> {
        if self.match_sse_response(input) == LlmProviderMatch::NoMatch {
            return None;
        }
        let parsed_events = input
            .events
            .iter()
            .map(|event| self.parse_sse_event(*event))
            .collect::<Vec<_>>();
        parsed_events_to_response(
            self.provider_id(),
            &parsed_events,
            structured_token_usage(input.events.iter().filter_map(|event| event.json)),
            true,
        )
    }

    fn parse_sse_event(&self, event: LlmSseEvent<'_>) -> LlmParsedSseEvent {
        let Some(value) = event.json else {
            return LlmParsedSseEvent {
                done: event.done_marker,
                ..LlmParsedSseEvent::default()
            };
        };
        match event.event_type {
            Some("metadata") => LlmParsedSseEvent {
                model: value
                    .get("model")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                ..LlmParsedSseEvent::default()
            },
            Some("output") => LlmParsedSseEvent {
                content_text: value
                    .get("response")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                    .map(ToString::to_string),
                reasoning_text: value
                    .get("reasoning_content")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                    .map(ToString::to_string),
                ..LlmParsedSseEvent::default()
            },
            Some("done") => LlmParsedSseEvent {
                done: true,
                finish_reason: value
                    .get("finish_reason")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                ..LlmParsedSseEvent::default()
            },
            _ => LlmParsedSseEvent::default(),
        }
    }

    fn new_stream_parser(&self) -> Box<dyn LlmProviderResponseStreamParser + Send> {
        Box::new(StructuredJsonSseStreamParser::default())
    }
}

fn structured_json_sse_request(input: &LlmRequestParserInput<'_>) -> bool {
    let Some(object) = input.json.as_object() else {
        return false;
    };
    if !request_messages_have_text(object) {
        return false;
    }
    if request_model_field(input.json).is_some() {
        return true;
    }
    request_named_structured_model(input.json).is_some() && request_has_context_signal(object)
}

fn request_model(value: &Value) -> Option<&str> {
    request_named_structured_model(value).or_else(|| request_model_field(value))
}

fn request_named_structured_model(value: &Value) -> Option<&str> {
    value
        .get("model_name")
        .or_else(|| value.get("provider_model_name"))
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
}

fn request_model_field(value: &Value) -> Option<&str> {
    value
        .get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
}

fn request_messages_have_text(object: &Map<String, Value>) -> bool {
    object
        .get("messages")
        .and_then(Value::as_array)
        .is_some_and(|messages| messages.iter().any(message_has_text_content))
}

fn request_has_user_input(object: &Map<String, Value>) -> bool {
    string_field_has_value(object, "user_input")
}

fn request_has_context_signal(object: &Map<String, Value>) -> bool {
    request_has_user_input(object)
        || string_field_has_value(object, "session_id")
        || string_field_has_value(object, "conversation_id")
        || string_field_has_value(object, "config_name")
        || object.contains_key("tools")
}

fn string_field_has_value(object: &Map<String, Value>, key: &str) -> bool {
    object
        .get(key)
        .and_then(Value::as_str)
        .is_some_and(|text| !text.is_empty())
}

fn message_has_text_content(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.get("role").and_then(Value::as_str).is_some()
        && object.get("content").is_some_and(message_content_has_text)
}

fn message_content_has_text(content: &Value) -> bool {
    match content {
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => items.iter().any(|item| {
            item.get("type").and_then(Value::as_str) == Some("text")
                && item
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.is_empty())
        }),
        _ => false,
    }
}

struct StructuredJsonSseEvidence {
    has_metadata_model: bool,
    has_output_delta: bool,
}

impl StructuredJsonSseEvidence {
    fn from_input(input: LlmSseResponseInput<'_>) -> Self {
        let has_metadata_model = input.events.iter().any(|event| {
            event.event_type == Some("metadata")
                && event
                    .json
                    .and_then(|value| value.get("model"))
                    .and_then(Value::as_str)
                    .is_some()
        });
        let has_output_delta = input.events.iter().any(|event| {
            event.event_type == Some("output")
                && event
                    .json
                    .and_then(|value| value.get("response"))
                    .and_then(Value::as_str)
                    .is_some()
        });
        Self {
            has_metadata_model,
            has_output_delta,
        }
    }

    fn is_strong(&self) -> bool {
        self.has_metadata_model && self.has_output_delta
    }
}

fn parsed_events_to_response(
    provider_id: &'static str,
    parsed_events: &[LlmParsedSseEvent],
    token_usage: Option<LlmTokenUsage>,
    stream: bool,
) -> Option<LlmParsedResponse> {
    let mut content_chunks = Vec::new();
    let mut reasoning_chunks = Vec::new();
    let mut model = None;
    let mut done = false;
    for event in parsed_events {
        if model.is_none() {
            model = event.model.clone();
        }
        if let Some(content) = &event.content_text {
            push_non_empty_text(&mut content_chunks, content);
        }
        if let Some(reasoning) = &event.reasoning_text {
            push_non_empty_text(&mut reasoning_chunks, reasoning);
        }
        done |= event.done;
    }
    if content_chunks.is_empty() && reasoning_chunks.is_empty() && !done {
        return None;
    }
    Some(LlmParsedResponse {
        provider_id,
        model,
        content_text: (!content_chunks.is_empty()).then(|| content_chunks.join("")),
        reasoning_text: (!reasoning_chunks.is_empty()).then(|| reasoning_chunks.join("")),
        token_usage,
        chunk_count: content_chunks.len() + reasoning_chunks.len(),
        done,
        stream,
        ..LlmParsedResponse::default()
    })
}

/// This protocol carries token counts on the event object itself rather than
/// under a `usage` key, so it feeds each event straight to the shared field
/// mapping instead of keeping its own copy of the naming rules.
fn structured_token_usage<'a>(
    values: impl IntoIterator<Item = &'a Value>,
) -> Option<LlmTokenUsage> {
    values
        .into_iter()
        .filter_map(|value| value.as_object())
        .filter_map(token_usage_from_map)
        .last()
}

#[derive(Default)]
struct StructuredJsonSseStreamParser {
    accumulator: ParsedSseResponseAccumulator,
    token_usage: Option<LlmTokenUsage>,
}

impl LlmProviderResponseStreamParser for StructuredJsonSseStreamParser {
    fn observe_event(&mut self, event: LlmSseEvent<'_>) -> LlmParsedSseEvent {
        if event.event_type == Some("token_usage")
            && let Some(value) = event.json
            && let Some(object) = value.as_object()
        {
            self.token_usage = token_usage_from_map(object);
        }
        let parsed = StructuredJsonSseParser.parse_sse_event(event);
        self.accumulator.observe(&parsed);
        parsed
    }

    fn finish(&mut self) -> Option<LlmParsedResponse> {
        self.accumulator.finish(
            STRUCTURED_JSON_SSE_PROTOCOL_ID,
            self.token_usage.clone(),
            true,
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::structured_token_usage;

    /// This protocol reaches the shared field mapping through its own entry
    /// point. Testing only the `usage`-wrapper entry would leave exactly the
    /// divergence that motivated sharing the table unguarded.
    #[test]
    fn reads_anthropic_names_through_the_event_entry_point() {
        let events = [json!({
            "input_tokens": 100,
            "output_tokens": 50,
            "cache_read_input_tokens": 800,
            "cache_creation_input_tokens": 200
        })];

        let usage = structured_token_usage(events.iter()).expect("usage from event object");

        assert_eq!(usage.prompt_tokens, Some(100));
        assert_eq!(usage.cached_prompt_tokens, Some(800));
        assert_eq!(usage.cache_creation_tokens, Some(200));
    }

    /// Counts live on the event object itself here, with no `usage` wrapper,
    /// so a wrapped payload must not be mistaken for one.
    #[test]
    fn ignores_a_usage_wrapper_it_does_not_use() {
        let events = [json!({ "usage": { "prompt_tokens": 10 } })];
        assert!(structured_token_usage(events.iter()).is_none());
    }

    /// Every event is offered to the mapping and the last match wins, so a
    /// later event carrying counts replaces an earlier one.
    #[test]
    fn the_last_event_carrying_counts_wins() {
        let events = [
            json!({ "prompt_tokens": 1, "completion_tokens": 2 }),
            json!({ "content": "no counts here" }),
            json!({ "prompt_tokens": 30, "completion_tokens": 40 }),
        ];

        let usage = structured_token_usage(events.iter()).expect("usage");

        assert_eq!(usage.prompt_tokens, Some(30));
        assert_eq!(usage.completion_tokens, Some(40));
    }
}
